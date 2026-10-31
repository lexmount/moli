use moli_cookie_jar::StoredCookie;
use tokio::sync::mpsc;

use super::{CdpCookieSnapshot, CookieProfileCommit, SharedCookieProfile};

pub(super) fn spawn_checkpoint_worker(
    cookie_profile: SharedCookieProfile,
    mut checkpoint_baseline: Vec<StoredCookie>,
    mut checkpoint_rx: mpsc::UnboundedReceiver<CdpCookieSnapshot>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(snapshot) = checkpoint_rx.recv().await {
            let Some(checkpoint_cookies) = snapshot.into_profile_backed_cookies() else {
                continue;
            };
            let commit =
                CookieProfileCommit::new(checkpoint_baseline.clone(), checkpoint_cookies.clone());
            let profile = cookie_profile.clone();
            match tokio::task::spawn_blocking(move || profile.commit_and_save(commit)).await {
                Ok(Ok(())) => checkpoint_baseline = checkpoint_cookies,
                Ok(Err(error)) => {
                    tracing::warn!(?error, "failed to persist CDP owner cookie checkpoint");
                }
                Err(error) => {
                    tracing::warn!(?error, "CDP owner cookie checkpoint worker panicked");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use moli_cookie_jar::{StoredCookie, StoredCookieSameSite, StoredCookieSourceScheme};

    use super::*;

    fn stored_cookie(name: &str, value: &str) -> StoredCookie {
        StoredCookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: "example.com".to_owned(),
            host_only: false,
            path: "/".to_owned(),
            secure: false,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Unspecified,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::NonSecure,
            source_port: -1,
            creation_index: 0,
            creation_time: None,
            last_access_index: 0,
        }
    }

    #[tokio::test]
    async fn checkpoint_worker_advances_cookie_delta_baseline() {
        let cookie_profile = SharedCookieProfile::new(Vec::new(), Vec::new());
        let (checkpoint_tx, checkpoint_rx) = mpsc::unbounded_channel();
        let worker = spawn_checkpoint_worker(cookie_profile.clone(), Vec::new(), checkpoint_rx);

        checkpoint_tx
            .send(CdpCookieSnapshot::from_profile_backed_cookies(Some(vec![
                stored_cookie("sid", "first"),
            ])))
            .expect("send added-cookie checkpoint");
        checkpoint_tx
            .send(CdpCookieSnapshot::from_profile_backed_cookies(Some(
                Vec::new(),
            )))
            .expect("send deleted-cookie checkpoint");
        drop(checkpoint_tx);
        worker.await.expect("checkpoint worker");

        assert!(
            cookie_profile.snapshot().is_empty(),
            "the second checkpoint must remove a cookie created by the first checkpoint"
        );
    }
}
