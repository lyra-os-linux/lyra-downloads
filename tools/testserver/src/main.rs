#[tokio::main]
async fn main() -> std::io::Result<()> {
    let bind = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8765".into());
    let server = lyra_downloads_testserver::start_on(&bind).await?;
    eprintln!("servidor de teste em http://{}", server.addr);
    std::future::pending::<()>().await;
    Ok(())
}
