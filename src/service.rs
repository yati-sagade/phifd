use std::str;
use bytes::BytesMut;
use tokio_io::codec::{Encoder, Decoder, Framed};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_proto::pipeline::ServerProto;
use tokio_service::Service;
use futures::{future, Future};
use tokio_proto::TcpServer;

use proto::msg::PingMsg;

pub enum Msg {
    Ping(PingMsg),
}

