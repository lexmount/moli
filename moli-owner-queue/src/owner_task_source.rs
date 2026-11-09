use std::collections::VecDeque;

use crate::OwnerWakeQueue;

#[derive(Debug)]
pub struct OwnerTaskSource<T> {
    tasks: VecDeque<T>,
    parser_boundary_wake: OwnerWakeQueue<T>,
    wake: OwnerWakeQueue<T>,
}

impl<T> Default for OwnerTaskSource<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> OwnerTaskSource<T> {
    pub fn new() -> Self {
        Self {
            tasks: VecDeque::new(),
            parser_boundary_wake: OwnerWakeQueue::new(),
            wake: OwnerWakeQueue::new(),
        }
    }

    pub fn sender(&self) -> tokio::sync::mpsc::UnboundedSender<T> {
        self.wake.sender()
    }

    pub fn parser_boundary_sender(&self) -> tokio::sync::mpsc::UnboundedSender<T> {
        self.parser_boundary_wake.sender()
    }

    pub fn enqueue_local(&mut self, item: T) {
        self.tasks.push_back(item);
    }

    pub fn enqueue_parser_boundary_local(&mut self, item: T) {
        self.tasks.push_front(item);
    }

    /// Materializes work that was posted since the previous owner turn.
    ///
    /// Callers use this at an outer scheduler boundary before probing the local
    /// runnable queue. It does not wait for future work and therefore does not
    /// turn a pending producer into a document-processing blocker.
    pub fn accept_ready_wakes(&mut self) {
        self.drain_wake();
    }

    pub fn extend_local<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = T>,
    {
        self.tasks.extend(items);
    }

    pub fn clear_local(&mut self) {
        self.drain_wake();
        self.tasks.clear();
    }

    pub fn with_tasks_mut<R>(&mut self, f: impl FnOnce(&mut VecDeque<T>) -> R) -> R {
        self.drain_wake();
        f(&mut self.tasks)
    }

    /// Inspect payloads already accepted into the owner-local queue without
    /// draining producer wakes.
    pub fn with_local_tasks<R>(&self, f: impl FnOnce(&VecDeque<T>) -> R) -> R {
        f(&self.tasks)
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.drain_wake();
        self.tasks.pop_front()
    }

    pub fn pop_front_local_only(&mut self) -> Option<T> {
        self.tasks.pop_front()
    }

    pub fn front(&mut self) -> Option<&T> {
        self.drain_wake();
        self.tasks.front()
    }

    pub fn front_local_only(&self) -> Option<&T> {
        self.tasks.front()
    }

    pub fn is_empty(&mut self) -> bool {
        self.drain_wake();
        self.tasks.is_empty()
    }

    pub fn is_empty_local_only(&self) -> bool {
        self.tasks.is_empty()
    }

    pub async fn wait_for_arrival(&mut self) -> bool {
        self.drain_wake();
        if !self.tasks.is_empty() {
            return true;
        }
        match self.wake.recv().await {
            Some(task) => {
                self.tasks.push_back(task);
                self.drain_wake();
                true
            }
            None => false,
        }
    }

    pub async fn wait_for_wake_arrival(&mut self) -> bool {
        tokio::select! {
            biased;
            item = self.parser_boundary_wake.recv() => {
                match item {
                    Some(task) => {
                        self.tasks.push_front(task);
                        self.drain_wake();
                        true
                    }
                    None => match self.wake.recv().await {
                        Some(task) => {
                            self.tasks.push_back(task);
                            self.drain_wake();
                            true
                        }
                        None => false,
                    }
                }
            }
            item = self.wake.recv() => {
                match item {
                    Some(task) => {
                        self.tasks.push_back(task);
                        self.drain_wake();
                        true
                    }
                    None => match self.parser_boundary_wake.recv().await {
                        Some(task) => {
                            self.tasks.push_front(task);
                            self.drain_wake();
                            true
                        }
                        None => false,
                    }
                }
            }
        }
    }

    pub async fn wait_for_local_wake_arrival(&mut self) -> bool {
        self.wait_for_arrival().await
    }

    fn drain_wake(&mut self) {
        let mut front = VecDeque::new();
        self.parser_boundary_wake.try_drain_into(&mut front);
        if !front.is_empty() {
            front.append(&mut self.tasks);
            self.tasks = front;
        }
        self.wake.try_drain_into(&mut self.tasks);
    }
}

#[cfg(test)]
mod tests {
    use super::OwnerTaskSource;

    #[test]
    fn local_and_woken_items_share_fifo_order() {
        let mut source = OwnerTaskSource::new();
        source.enqueue_local(1);
        let sender = source.sender();
        sender.send(2).unwrap();

        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
        assert!(source.pop_front().is_none());
    }

    #[test]
    fn enqueue_parser_boundary_local_precedes_existing_work() {
        let mut source = OwnerTaskSource::new();
        source.enqueue_local(2);
        source.enqueue_parser_boundary_local(1);

        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
    }

    #[tokio::test]
    async fn wait_for_arrival_observes_future_wake() {
        let mut source = OwnerTaskSource::new();
        let sender = source.sender();
        tokio::spawn(async move {
            let _ = sender.send(7);
        });

        assert!(source.wait_for_arrival().await);
        assert_eq!(source.pop_front(), Some(7));
    }

    #[tokio::test]
    async fn wait_for_local_wake_arrival_drains_pending_parser_boundary_wake() {
        let mut source = OwnerTaskSource::new();
        source.enqueue_local(2);
        source.parser_boundary_sender().send(1).unwrap();

        assert!(source.wait_for_local_wake_arrival().await);
        assert_eq!(source.pop_front_local_only(), Some(1));
        assert_eq!(source.pop_front_local_only(), Some(2));
    }

    #[test]
    fn parser_boundary_wake_items_precede_existing_local_and_regular_wake_work() {
        let mut source = OwnerTaskSource::new();
        source.enqueue_local(2);
        source.sender().send(3).unwrap();
        source.parser_boundary_sender().send(1).unwrap();

        assert_eq!(source.pop_front(), Some(1));
        assert_eq!(source.pop_front(), Some(2));
        assert_eq!(source.pop_front(), Some(3));
    }

    #[test]
    fn owner_turn_can_accept_ready_wakes_without_waiting() {
        let mut source = OwnerTaskSource::new();
        source.enqueue_local(2);
        source.sender().send(3).unwrap();
        source.parser_boundary_sender().send(1).unwrap();

        source.accept_ready_wakes();

        assert_eq!(source.pop_front_local_only(), Some(1));
        assert_eq!(source.pop_front_local_only(), Some(2));
        assert_eq!(source.pop_front_local_only(), Some(3));
    }
}
