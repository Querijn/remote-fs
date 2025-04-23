use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::OwnedReadHalf,
};

#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum MessageType {
    Sync { files: HashMap<String, Vec<u8>> },
    CreateEvent { path: String, contents: Vec<u8> },
    ModifyEvent { path: String, contents: Vec<u8> },
    DeleteEvent { path: String },
    MoveEvent { old_path: String, new_path: String },
}

pub(crate) fn compose_data_message(event: &MessageType) -> Vec<u8> {
    let event_data = serde_binary::to_vec(&event, serde_binary::binary_stream::Endian::Big);
    if event_data.is_err() {
        eprintln!("Failed to serialize event: {:?}", event);
        return vec![];
    }

    let event_data = event_data.unwrap();
    let msg_len = event_data.len();
    let len = (msg_len as u32).to_be_bytes();

    let mut data = Vec::with_capacity(4 + msg_len);
    data.extend_from_slice(&len);
    data.extend_from_slice(&event_data);
    data
}

pub(crate) fn parse_msg(data: &[u8]) -> Result<MessageType, serde_binary::Error> {
    let event: MessageType = serde_binary::from_slice(data, serde_binary::binary_stream::Endian::Big)?;
    Ok(event)
}

pub(crate) async fn read_msg(reader: &mut OwnedReadHalf) -> Result<MessageType, MessageError> {
    let mut len_buf = [0u8; 4];
    let bytes_read = reader.read(&mut len_buf).await;
    if let Err(_) = bytes_read {
        return Err(MessageError::parse_error("Failed to read length"));
    }
    if bytes_read.unwrap() == 0 {
        return Err(MessageError::disconnect_error("Disconnected"));
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    let mut msg_buf = vec![0u8; len];
    if reader.read_exact(&mut msg_buf).await.is_err() {
        return Err(MessageError::parse_error("Failed to read length"));
    }

    let result = parse_msg(&msg_buf);
    if result.is_err() {
        eprintln!("Failed to parse message: {:?}", result);
        return Err(MessageError::parse_error("Failed to parse message"));
    }

    let result = result.unwrap();
    eprintln!("Received event ({} bytes)", msg_buf.len() + 4);
    Ok(result)
}

pub(crate) async fn write_msg<T>(writer: &mut T, msg: &MessageType) -> Result<(), MessageError>
where
    T: AsyncWriteExt + Unpin,
{
    let msg_data = compose_data_message(msg);
    if msg_data.is_empty() {
        return Err(MessageError::parse_error("Failed to serialize message"));
    }

    if writer.write_all(&msg_data).await.is_err() {
        return Err(MessageError::disconnect_error("Failed to write message"));
    }

    eprintln!("Sent event ({} bytes)", msg_data.len());
    Ok(())
}

pub(crate) struct MessageError {
    msg: String,
    is_disconnected: bool,
}

impl MessageError {
    pub fn parse_error(msg: &str) -> Self {
        MessageError {
            msg: msg.to_string(),
            is_disconnected: false,
        }
    }

    pub fn disconnect_error(msg: &str) -> Self {
        MessageError {
            msg: msg.to_string(),
            is_disconnected: true,
        }
    }

    #[allow(dead_code)]
    pub fn is_disconnected(&self) -> bool {
        self.is_disconnected
    }
}

impl fmt::Debug for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageError")
            .field("msg", &self.msg)
            .field("is_disconnected", &self.is_disconnected)
            .finish()
    }
}
