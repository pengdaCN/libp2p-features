use futures::StreamExt;
use libp2p::build_multiaddr;
use libp2p::identify;
use libp2p::identify::Event;
use libp2p::identity::Keypair;
use libp2p::kad::{self, Kademlia};
use libp2p::ping;
use libp2p::relay;
use libp2p::swarm;
use libp2p::swarm::SwarmEvent;
use libp2p::swarm::{AddressScore, NetworkBehaviour};

#[tokio::main]
async fn main() {
    let key = Keypair::generate_ed25519();

    let tcp_transport = libp2p_feature::build_tcp_transport(&key);
    let peer = key.public().to_peer_id();
    let behavior = {
        let identify = identify::Behaviour::new(identify::Config::new(
            String::from("/relay/ping/0.1"),
            key.public(),
        ));
        let ping = ping::Behaviour::new(Default::default());
        let relay = relay::Behaviour::new(peer, Default::default());
        let kad = Kademlia::new(peer, kad::store::MemoryStore::new(peer));

        Behavior {
            identify,
            ping,
            relay,
            kad,
        }
    };

    let mut swarm = swarm::SwarmBuilder::with_tokio_executor(tcp_transport, behavior, peer).build();
    let listen_addr = build_multiaddr!(Ip4([127, 0, 0, 1]), Tcp(9000u16));
    let listen_id = swarm
        .listen_on(listen_addr.clone())
        .expect("listen addr failed");

    // swarm.add_external_address(listen_addr, AddressScore::Finite(1));
    println!("peer id ({peer})");

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } if listener_id == listen_id => {
                println!("external address: {address}");

                swarm.add_external_address(address, AddressScore::Infinite);
            }

            SwarmEvent::Behaviour(BehaviorEvent::Identify(event)) => match event {
                Event::Received { peer_id, info } => {
                    swarm
                        .behaviour_mut()
                        .kad
                        .add_address(&peer_id, info.observed_addr);
                }
                _ => {}
            },

            event => println!("relay event {event:?}"),
        }
    }
}

#[derive(NetworkBehaviour)]
struct Behavior {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay: relay::Behaviour,
    kad: Kademlia<kad::store::MemoryStore>,
}
