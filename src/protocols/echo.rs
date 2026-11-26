use iroh::{
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};

pub(crate) const ALPN: &[u8] = b"echo-testing/0.1.0\0";

#[derive(Debug, Clone)]
pub(crate) struct Echo;

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let bytes_sent = tokio::io::copy(&mut recv, &mut send).await?;

        tracing::debug!(%bytes_sent, "handled connection");

        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}
