use std::time::{Duration, Instant};

const BUDGET: Duration = Duration::from_secs(5);

pub(super) struct Notice {
    pub signal: i32,
    pub deadline: Instant,
}

pub(super) struct VideoTermination {
    #[cfg(unix)]
    relay: moli_process_signal::BoundedTermination,
}

impl VideoTermination {
    pub fn install() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            relay: moli_process_signal::BoundedTermination::install(BUDGET)?,
        })
    }

    pub async fn notified(&mut self) -> std::io::Result<Notice> {
        #[cfg(unix)]
        {
            let notice = self.relay.notified().await?;
            Ok(Notice {
                signal: notice.signal,
                deadline: notice.deadline,
            })
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await?;
            let deadline = Instant::now() + BUDGET;
            std::thread::Builder::new()
                .name("moli-signal".into())
                .spawn(move || {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    std::process::exit(130);
                })?;
            tokio::spawn(async {
                if tokio::signal::ctrl_c().await.is_ok() {
                    std::process::exit(130);
                }
            });
            Ok(Notice {
                signal: 2,
                deadline,
            })
        }
    }
}
