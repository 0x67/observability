use async_trait::async_trait;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use tokio::runtime::Handle;

/// Async reqwest client whose requests run on the export runtime. The SDK's batch threads
/// poll export futures with `futures_executor::block_on`, which has no reactor.
// holds a Handle, not the runtime: the global subscriber keeps this client alive until
// exit, and owning the runtime here would keep it running after every guard dropped
#[derive(Debug)]
pub(crate) struct RuntimeHttpClient {
    client: reqwest::Client,
    runtime: Handle,
}

impl RuntimeHttpClient {
    pub(crate) fn new(client: reqwest::Client, runtime: Handle) -> Self {
        Self { client, runtime }
    }
}

#[async_trait]
impl HttpClient for RuntimeHttpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let client = self.client.clone();
        self.runtime
            .spawn(async move { client.send_bytes(request).await })
            .await?
    }
}
