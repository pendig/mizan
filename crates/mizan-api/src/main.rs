#[tokio::main]
async fn main() -> mizan_core::AppResult<()> {
    mizan_api::run_from_env().await
}
