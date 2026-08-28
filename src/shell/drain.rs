use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Read `reader` to EOF while retaining at most `limit` bytes.
pub async fn drain_limited<R>(mut reader: R, limit: usize) -> io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    const BUFFER_BYTES: usize = 8 * 1024;
    let mut retained = Vec::with_capacity(limit.min(BUFFER_BYTES));
    let mut truncated = false;
    let mut buffer = [0u8; BUFFER_BYTES];

    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let room = limit.saturating_sub(retained.len());
        let keep = read.min(room);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }

    Ok((retained, truncated))
}

#[cfg(test)]
mod tests;
