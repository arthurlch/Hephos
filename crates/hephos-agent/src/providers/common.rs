use futures::StreamExt;
use hephos_core::Error;

use crate::message::TextStream;

pub(crate) fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(crate) fn parse_sse_stream<F>(response: reqwest::Response, extractor: F) -> TextStream
where
    F: Fn(&str) -> Option<String> + Send + Sync + 'static,
{
    let stream = async_stream::try_stream! {
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_res) = byte_stream.next().await {
            let chunk = chunk_res.map_err(|e| Error::internal(format!("network error in stream: {e}")))?;
            let text = std::str::from_utf8(&chunk)
                .map_err(|e| Error::internal(format!("invalid utf-8 in sse stream: {e}")))?;
            buffer.push_str(text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer.drain(..=pos);

                if let Some(delta) = extractor(&line) {
                    if !delta.is_empty() {
                        yield delta;
                    }
                }
            }
        }
    };
    Box::pin(stream)
}
