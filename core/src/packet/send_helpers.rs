use std::io::Cursor;

use binrw::BinWrite;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::{
    common::RECEIVE_BUFFER_SIZE, common::timestamp_msecs, config::get_config,
    ipc::kawari::CustomIpcSegment,
};

use super::{
    CompressionType, ConnectionState, ConnectionType, PacketHeader, PacketSegment,
    ReadWriteIpcSegment, SegmentData, SegmentType, compression::compress, parse_packet,
};

fn first_custom_ipc_response_segment(
    segments: &[PacketSegment<CustomIpcSegment>],
) -> Option<CustomIpcSegment> {
    segments.iter().find_map(|segment| match &segment.data {
        SegmentData::KawariIpc(data) => Some(data.clone()),
        _ => None,
    })
}

pub async fn send_packet<T: ReadWriteIpcSegment>(
    socket: &mut TcpStream,
    state: &mut ConnectionState,
    connection_type: ConnectionType,
    compression_type: CompressionType,
    segments: &[PacketSegment<T>],
) {
    let (data, uncompressed_size) = compress(state, &compression_type, segments);
    let size = std::mem::size_of::<PacketHeader>() + data.len();

    let header = PacketHeader {
        timestamp: timestamp_msecs(),
        size: size as u32,
        connection_type,
        segment_count: segments.len() as u16,
        compression_type,
        uncompressed_size: uncompressed_size as u32,
        ..Default::default()
    };

    let mut cursor = Cursor::new(Vec::with_capacity(size));
    header.write_le(&mut cursor).unwrap();
    std::io::Write::write_all(&mut cursor, &data).unwrap();

    let buffer = cursor.into_inner();
    assert!(buffer.len() < RECEIVE_BUFFER_SIZE);

    if let Err(e) = socket.write_all(&buffer).await {
        tracing::warn!("Failed to send packet: {e}");
    }
}

pub async fn send_keep_alive<T: ReadWriteIpcSegment>(
    socket: &mut TcpStream,
    state: &mut ConnectionState,
    connection_type: ConnectionType,
    id: u32,
    timestamp: u32,
) {
    let response_packet: PacketSegment<T> = PacketSegment {
        segment_type: SegmentType::KeepAliveResponse,
        data: SegmentData::KeepAliveResponse { id, timestamp },
        ..Default::default()
    };
    send_packet(
        socket,
        state,
        connection_type,
        CompressionType::Uncompressed,
        &[response_packet],
    )
    .await;
}

/// Sends a custom IPC packet to the world server, meant for private server-to-server communication.
/// Returns the first custom IPC segment returned.
pub async fn send_custom_world_packet(segment: CustomIpcSegment) -> Option<CustomIpcSegment> {
    let config = get_config();

    let addr = config.world.get_public_socketaddr();

    let mut stream = match TcpStream::connect(addr).await {
        Ok(stream) => stream,
        Err(err) => {
            tracing::warn!("Failed to connect to custom world IPC at {addr}: {err}");
            return None;
        }
    };

    let mut packet_state = ConnectionState::None;

    let segment: PacketSegment<CustomIpcSegment> = PacketSegment {
        segment_type: SegmentType::KawariIpc,
        data: SegmentData::KawariIpc(segment),
        ..Default::default()
    };

    send_packet(
        &mut stream,
        &mut packet_state,
        ConnectionType::KawariIpc,
        CompressionType::Uncompressed,
        &[segment],
    )
    .await;

    // read response
    let mut buf = vec![0; RECEIVE_BUFFER_SIZE];
    let n = match stream.read(&mut buf).await {
        Ok(n) => n,
        Err(err) => {
            tracing::warn!("Failed to read custom world IPC response: {err}");
            return None;
        }
    };

    if n == 0 {
        tracing::warn!("Custom world IPC connection closed without a response.");
        return None;
    }

    let segments = parse_packet::<CustomIpcSegment>(&buf[..n], &mut packet_state);
    if segments.is_empty() {
        tracing::warn!("Failed to parse custom world IPC response.");
        return None;
    }

    let response = first_custom_ipc_response_segment(&segments);
    if response.is_none() {
        tracing::warn!("Custom world IPC response did not contain a Kawari IPC segment.");
    }

    response
}

#[cfg(test)]
mod tests {
    use crate::ipc::kawari::{CustomIpcData, CustomIpcSegment};
    use crate::packet::{PacketSegment, SegmentData, SegmentType};

    #[test]
    fn first_custom_ipc_response_segment_returns_none_for_empty_segments() {
        assert!(super::first_custom_ipc_response_segment(&[]).is_none());
    }

    #[test]
    fn first_custom_ipc_response_segment_returns_first_custom_ipc_payload() {
        let expected = CustomIpcSegment::new(CustomIpcData::RequestHousingSummary {});
        let segments = vec![PacketSegment {
            segment_type: SegmentType::KawariIpc,
            data: SegmentData::KawariIpc(expected.clone()),
            ..Default::default()
        }];

        let actual = super::first_custom_ipc_response_segment(&segments)
            .expect("expected first custom IPC payload to be returned");
        match actual.data {
            CustomIpcData::RequestHousingSummary {} => {}
            other => panic!("unexpected payload returned: {other:?}"),
        }
    }

    #[test]
    fn first_custom_ipc_response_segment_skips_non_custom_segments() {
        let expected = CustomIpcSegment::new(CustomIpcData::RequestHousingSummary {});
        let segments = vec![
            PacketSegment {
                segment_type: SegmentType::KeepAliveResponse,
                data: SegmentData::KeepAliveResponse {
                    id: 7,
                    timestamp: 11,
                },
                ..Default::default()
            },
            PacketSegment {
                segment_type: SegmentType::KawariIpc,
                data: SegmentData::KawariIpc(expected.clone()),
                ..Default::default()
            },
        ];

        let actual = super::first_custom_ipc_response_segment(&segments)
            .expect("expected later custom IPC payload to be returned");
        match actual.data {
            CustomIpcData::RequestHousingSummary {} => {}
            other => panic!("unexpected payload returned: {other:?}"),
        }
    }
}
