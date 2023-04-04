pub mod message;

use libp2p::core::identity::Keypair;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::transport::Boxed;
use libp2p::core::upgrade;
use libp2p::noise::NoiseAuthenticated;
use libp2p::tcp::tokio::Transport as TcpTransport;
use libp2p::{yamux, PeerId, Transport};

pub fn build_tcp_transport(k: &Keypair) -> Boxed<(PeerId, StreamMuxerBox)> {
    TcpTransport::default()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseAuthenticated::xx(k).expect("build transport failed because of noise"))
        .multiplex(yamux::YamuxConfig::default())
        .boxed()
}
