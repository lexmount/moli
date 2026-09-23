use crate::parser_script::item::ParserClassicScriptRunnerItem;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptRunnerQueue<C> {
    scripts: ParserBlockingClassicScriptQueue<ParserClassicScriptRunnerItem<C>>,
}

#[derive(Debug, Clone)]
struct ParserBlockingClassicScriptQueue<T> {
    current: Option<T>,
    queued_after_current: VecDeque<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserBlockingClassicScriptQueueTransition {
    KeepCurrent,
    FinishCurrent,
}

impl<C> ParserClassicScriptRunnerQueue<C> {
    pub(crate) fn empty() -> Self {
        Self {
            scripts: ParserBlockingClassicScriptQueue::empty(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(scripts: impl IntoIterator<Item = ParserClassicScriptRunnerItem<C>>) -> Self {
        Self {
            scripts: ParserBlockingClassicScriptQueue::new(scripts),
        }
    }

    pub(crate) fn push(&mut self, script: ParserClassicScriptRunnerItem<C>) {
        self.scripts.push(script);
    }

    pub(crate) fn finish_current(&mut self) {
        self.scripts.finish_current();
    }

    pub(crate) fn take_current(&mut self) -> Option<ParserClassicScriptRunnerItem<C>> {
        let script = self.scripts.current.take()?;
        self.scripts.finish_current();
        Some(script)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    pub(crate) fn current(&self) -> Option<&ParserClassicScriptRunnerItem<C>> {
        self.scripts.current()
    }

    pub(crate) fn find_map<R>(
        &self,
        mut find: impl FnMut(&ParserClassicScriptRunnerItem<C>) -> Option<R>,
    ) -> Option<R> {
        self.scripts.iter().find_map(&mut find)
    }

    pub(crate) fn all_contexts_match(&self, mut predicate: impl FnMut(&C) -> bool) -> bool {
        self.scripts
            .iter()
            .all(|script| predicate(script.context()))
    }

    pub(crate) fn update_current_and_keep<R>(
        &mut self,
        update: impl FnOnce(&mut ParserClassicScriptRunnerItem<C>) -> Option<R>,
    ) -> Option<R> {
        self.update_current_with_transition(|script| {
            Some((
                update(script)?,
                ParserBlockingClassicScriptQueueTransition::KeepCurrent,
            ))
        })
    }

    pub(crate) fn update_current_and_advance<R>(
        &mut self,
        update: impl FnOnce(&mut ParserClassicScriptRunnerItem<C>) -> Option<R>,
    ) -> Option<R> {
        self.update_current_with_transition(|script| {
            Some((
                update(script)?,
                ParserBlockingClassicScriptQueueTransition::FinishCurrent,
            ))
        })
    }

    pub(crate) fn update_first_matching_and_keep<R>(
        &mut self,
        mut matches: impl FnMut(&ParserClassicScriptRunnerItem<C>) -> bool,
        update: impl FnOnce(&mut ParserClassicScriptRunnerItem<C>) -> Option<R>,
    ) -> Option<R> {
        let script = self.scripts.iter_mut().find(|script| matches(script))?;
        update(script)
    }

    fn update_current_with_transition<R>(
        &mut self,
        update: impl FnOnce(
            &mut ParserClassicScriptRunnerItem<C>,
        ) -> Option<(R, ParserBlockingClassicScriptQueueTransition)>,
    ) -> Option<R> {
        self.scripts.update_current(update)
    }
}

impl<T> ParserBlockingClassicScriptQueue<T> {
    fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    fn new(scripts: impl IntoIterator<Item = T>) -> Self {
        let mut scripts = scripts.into_iter();
        Self {
            current: scripts.next(),
            queued_after_current: scripts.collect(),
        }
    }

    fn push(&mut self, script: T) {
        if self.current.is_none() {
            self.current = Some(script);
        } else {
            self.queued_after_current.push_back(script);
        }
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.current.is_none() && self.queued_after_current.is_empty()
    }

    fn current(&self) -> Option<&T> {
        self.current.as_ref()
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.current.iter().chain(self.queued_after_current.iter())
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.current
            .iter_mut()
            .chain(self.queued_after_current.iter_mut())
    }

    fn current_mut(&mut self) -> Option<&mut T> {
        self.current.as_mut()
    }

    fn finish_current(&mut self) {
        self.current = self.queued_after_current.pop_front();
    }

    fn update_current<R>(
        &mut self,
        update: impl FnOnce(&mut T) -> Option<(R, ParserBlockingClassicScriptQueueTransition)>,
    ) -> Option<R> {
        let (result, transition) = update(self.current_mut()?)?;
        if transition == ParserBlockingClassicScriptQueueTransition::FinishCurrent {
            self.finish_current();
        }
        Some(result)
    }
}
