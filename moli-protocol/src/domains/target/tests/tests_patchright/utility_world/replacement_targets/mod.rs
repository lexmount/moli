use super::*;

async fn patchright_replacement_targets_large_stack<F, Fut>(build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    let result = std::thread::Builder::new()
        .name("patchright-replacement-targets".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("large-stack patchright replacement-targets test runtime should build")
                .block_on(build());
        })
        .expect("large-stack patchright replacement-targets test thread should spawn")
        .join();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

mod thin_variants;
