use rustyred_thg_core::StampedBatch;
use serde::{Deserialize, Serialize};
use theorem_copresence::PeerEvent;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{FederationError, Result};

pub const MAX_DELTA_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FederationFrame {
    StructuredDelta {
        scope: String,
        batch: StampedBatch,
    },
    TextUpdate {
        scope: String,
        region_id: String,
        state_vector_v1: Vec<u8>,
        update_v1: Vec<u8>,
    },
    Presence {
        scope: String,
        event: PeerEvent,
    },
}

pub fn encode_frame(frame: &FederationFrame) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(frame)?;
    if bytes.len() > MAX_DELTA_FRAME_BYTES {
        return Err(FederationError::FrameTooLarge {
            actual: bytes.len(),
            limit: MAX_DELTA_FRAME_BYTES,
        });
    }
    Ok(bytes)
}

pub fn decode_frame(bytes: &[u8]) -> Result<FederationFrame> {
    if bytes.len() > MAX_DELTA_FRAME_BYTES {
        return Err(FederationError::FrameTooLarge {
            actual: bytes.len(),
            limit: MAX_DELTA_FRAME_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(Into::into)
}

pub async fn write_frame<W>(writer: &mut W, frame: &FederationFrame) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let bytes = encode_frame(frame)?;
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R>(reader: &mut R) -> Result<FederationFrame>
where
    R: AsyncRead + Unpin,
{
    let mut len = [0_u8; 4];
    reader.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_DELTA_FRAME_BYTES {
        return Err(FederationError::FrameTooLarge {
            actual: len,
            limit: MAX_DELTA_FRAME_BYTES,
        });
    }
    let mut bytes = vec![0_u8; len];
    reader.read_exact(&mut bytes).await?;
    decode_frame(&bytes)
}
