use futures::StreamExt;
use hephos_core::{Error, Result};

use crate::message::TextStream;

pub(crate) fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|e| {
            // Extremely rare (TLS backend init failure). Warn loudly: the fallback
            // client has NO request timeout, so a hung provider socket would then be
            // bounded only by the agent's wall-clock `Limits.timeout`.
            tracing::error!(error = %e, "failed to build timeout-configured HTTP client; falling back to an untimed client");
            reqwest::Client::new()
        })
}

/// Upper bound on buffer capacity for SSE line parsing (2 MiB) to prevent memory exhaustion DoS.
pub const MAX_SSE_BUFFER_BYTES: usize = 2 * 1024 * 1024;

/// Extract complete lines ending in `\n` from an accumulated byte buffer,
/// respecting UTF-8 boundaries and capping buffer capacity.
pub(crate) fn extract_sse_lines(buffer: &mut Vec<u8>, max_bytes: usize) -> Result<Vec<String>> {
    if buffer.len() > max_bytes {
        return Err(Error::internal("SSE stream buffer limit exceeded"));
    }

    let mut lines = Vec::new();
    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let line_bytes = &buffer[..pos];
        let line_str = std::str::from_utf8(line_bytes)
            .map_err(|e| Error::internal(format!("invalid utf-8 in sse stream: {e}")))?;
        lines.push(line_str.trim_end_matches('\r').to_string());
        buffer.drain(..=pos);
    }
    Ok(lines)
}

pub(crate) fn parse_sse_stream<F>(response: reqwest::Response, extractor: F) -> TextStream
where
    F: Fn(&str) -> Option<String> + Send + Sync + 'static,
{
    let stream = async_stream::try_stream! {
        let mut byte_stream = response.bytes_stream();
        let mut buffer = Vec::new();

        while let Some(chunk_res) = byte_stream.next().await {
            let chunk = chunk_res.map_err(|e| Error::internal(format!("network error in stream: {e}")))?;
            buffer.extend_from_slice(&chunk);

            let lines = extract_sse_lines(&mut buffer, MAX_SSE_BUFFER_BYTES)?;
            for line in lines {
                if let Some(delta) = extractor(&line) {
                    if !delta.is_empty() {
                        yield delta;
                    }
                }
            }
        }

        if !buffer.is_empty() {
            let line_str = std::str::from_utf8(&buffer)
                .map_err(|e| Error::internal(format!("invalid utf-8 in trailing sse stream: {e}")))?;
            let line = line_str.trim_end_matches('\r');
            if let Some(delta) = extractor(line) {
                if !delta.is_empty() {
                    yield delta;
                }
            }
        }
    };
    Box::pin(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_sse_lines_handles_split_utf8_sequences() {
        let mut buffer = Vec::new();

        // 4-byte UTF-8 emoji "🚀" is bytes [0xF0, 0x9F, 0x9A, 0x80]
        // Chunk 1 has first 2 bytes, no newline yet
        buffer.extend_from_slice(b"data: hello \xf0\x9f");
        let lines1 = extract_sse_lines(&mut buffer, MAX_SSE_BUFFER_BYTES).unwrap();
        assert!(lines1.is_empty());
        assert_eq!(buffer.len(), 14);

        // Chunk 2 arrives with remaining 2 bytes and a newline
        buffer.extend_from_slice(b"\x9a\x80\n");
        let lines2 = extract_sse_lines(&mut buffer, MAX_SSE_BUFFER_BYTES).unwrap();
        assert_eq!(lines2.len(), 1);
        assert_eq!(lines2[0], "data: hello 🚀");
        assert!(buffer.is_empty());
    }

    #[test]
    fn extract_sse_lines_enforces_buffer_capacity_limit() {
        let mut buffer = vec![b'a'; 100];
        // Limit of 50 bytes should reject 100 bytes buffer
        let res = extract_sse_lines(&mut buffer, 50);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("SSE stream buffer limit exceeded")
        );
    }
}
