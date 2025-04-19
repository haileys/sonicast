pub struct MpdStatus {
    // conn: Conn,
    task: tokio::task::JoinHandle<()>,
    watch: Arc<Watch>,
}

struct Watch {

}

impl MpdIdle {
    pub async fn connect(config: &Config) -> Result<Self> {
        let conn = Mpd::connect(config).await?;
        let task = tokio::task::spawn(async move {
            loop {
                match conn.idle().await {

                }
            }
        });
    }
}
