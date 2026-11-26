use iroh::{
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use n0_error::StdResultExt;

pub(crate) const ALPN: &[u8] = b"reverse-testing/0.1.0\0";

#[derive(Debug, Clone)]
pub(crate) struct Reverse;

impl ProtocolHandler for Reverse {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;

        let msg = recv.read_to_end(10).await.std_context("read")?;
        tracing::debug!("read message");

        let response_message = msg.into_iter().rev().collect::<Vec<_>>();
        send.write_all(&response_message).await.std_context("write")?;

        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}
