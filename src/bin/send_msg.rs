use futures::StreamExt;
use libp2p::core::transport::{upgrade, OrTransport};
use libp2p::identity::Keypair;
use libp2p::kad::{self, Kademlia};
use libp2p::multiaddr::Protocol;
use libp2p::noise::NoiseAuthenticated;
use libp2p::relay::client as relay_client;
use libp2p::request_response::ProtocolSupport;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::tcp::tokio::Transport as TcpTransport;
use libp2p::{identify, swarm, yamux, Swarm, Transport};
use libp2p::{ping, Multiaddr};
use libp2p::{request_response, PeerId};
use libp2p_feature::message;
use libp2p_feature::message::{Message, Receipt};
use std::iter;
use tokio::io;
use tokio::io::AsyncBufReadExt;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;

#[tokio::main]
async fn main() {
    let key = Keypair::generate_ed25519();

    let peer = key.public().to_peer_id();
    let (relay_transport, relay_behavior) = relay_client::new(peer);
    let transport = {
        let tcp_transport = TcpTransport::default().boxed();

        OrTransport::new(tcp_transport, relay_transport)
            .upgrade(upgrade::Version::V1)
            .authenticate(
                NoiseAuthenticated::xx(&key).expect("build transport failed because of noise"),
            )
            .multiplex(yamux::YamuxConfig::default())
            .boxed()
    };

    let behavior = {
        let identify = identify::Behaviour::new(identify::Config::new(
            String::from("/send_msg/ping/0.1"),
            key.public(),
        ));
        let ping = ping::Behaviour::new(Default::default());
        let request_response = request_response::Behaviour::new(
            message::Codec,
            iter::once((message::MsgProto, ProtocolSupport::Full)),
            Default::default(),
        );
        let relay = relay_behavior;
        let kad = Kademlia::new(peer, kad::store::MemoryStore::new(peer));

        Behavior {
            identify,
            ping,
            request_response,
            relay,
            kad,
        }
    };

    let mut swarm =
        swarm::SwarmBuilder::with_tokio_executor(transport, behavior, key.public().to_peer_id())
            .build();

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(1);
    tokio::spawn(handle_command(event_tx));

    println!("peer {}", key.public().to_peer_id());
    loop {
        select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(event) => {
                        match event {
                            BehaviorEvent::Ping(_) => {},
                            BehaviorEvent::Relay(e) => println!("relay event {e:?}"),
                            BehaviorEvent::Identify(e) => {println!("relay event {e:?}")},
                            BehaviorEvent::RequestResponse(e) => {
                                println!("request-response event {e:?}");
                                handle_event_request_response(&mut swarm, e);
                            }
                            BehaviorEvent::Kad(e) => println!("kad event {e:?}"),
                        }

                    },
                    event => println!("other event {event:?}"),

                }
                // println!("send_msg event {event:?}");
            }
            event = event_rx.recv() => {
                if let Some(event) = event {
                    println!("recv event >>> {event:?}");
                    handle_event(event, &mut swarm)
                } else {
                    break
                }
            }
        }
    }
}

fn handle_event_request_response(
    swarm: &mut Swarm<Behavior>,
    event: request_response::Event<Message, Receipt>,
) {
    use request_response::Event;

    match event {
        Event::Message {
            message:
                request_response::Message::Request {
                    request, channel, ..
                },
            ..
        } => {
            println!(
                "recv msg >>> {}",
                String::from_utf8(request.0).expect("invalid string")
            );
            swarm
                .behaviour_mut()
                .request_response
                .send_response(channel, Receipt::Pong)
                .expect("send response failed");
        }
        _ => {}
    }
}

async fn handle_command(event_tx: Sender<Event>) {
    let buf_reader = io::BufReader::new(io::stdin());
    let mut lines = buf_reader.lines();

    loop {
        let Some(line) = lines.next_line().await.ok().flatten() else {
            continue;
        };

        let mut args = line.split_whitespace();

        let Some(command) = args.next() else {
            continue;
        };

        match command.to_lowercase().as_str() {
            "listen" => {
                let Some(addr) = args.next() else {
                    continue;
                };

                let addr = addr
                    .parse::<Multiaddr>()
                    .expect("parse relay address for listen failed");

                event_tx
                    .send(Event::Listen(addr))
                    .await
                    .expect("send event failed for listen");
            }

            "dial" => {
                let dst_peer = args.next().and_then(|x| x.parse::<PeerId>().ok());
                let dst_addr = args.next().and_then(|x| x.parse::<Multiaddr>().ok());

                if let (Some(dst_peer), Some(dst_addr)) = (dst_peer, dst_addr) {
                    event_tx
                        .send(Event::Dial {
                            peer: dst_peer,
                            addr: dst_addr,
                        })
                        .await
                        .expect("send event failed for dial");
                }
            }

            "send" => {
                let Some(dst_peer) = args.next().and_then(|x| x.parse::<PeerId>().ok()) else { continue; };
                let Some(content) = args.next().map(String::from) else { continue; };

                event_tx
                    .send(Event::Send {
                        peer: dst_peer,
                        content,
                    })
                    .await
                    .expect("send event failed for send");
            }

            "send_times" => {
                let Some(times) = args.next().and_then(|x| x.parse::<usize>().ok()) else { continue; };
                let Some(dst_peer) = args.next().and_then(|x| x.parse::<PeerId>().ok()) else { continue; };
                let Some(content) = args.next().map(String::from) else { continue; };

                for _ in 0..times {
                    event_tx
                        .send(Event::Send {
                            peer: dst_peer,
                            content: content.clone(),
                        })
                        .await
                        .expect("send event failed for send");
                }
            }

            "join" => {
                let peer = args.next().and_then(|x| x.parse::<PeerId>().ok());
                let addr = args.next().and_then(|x| x.parse::<Multiaddr>().ok());
                if let (Some(peer), Some(addr)) = (peer, addr) {
                    event_tx
                        .send(Event::Join { peer, addr })
                        .await
                        .expect("send event failed for join");
                }
            }

            _ => (),
        }
    }
}

fn handle_event(e: Event, swarm: &mut Swarm<Behavior>) {
    match e {
        Event::Send { peer, content } => {
            swarm
                .behaviour_mut()
                .request_response
                .send_request(&peer, message::Message(content.into()));
        }
        Event::Dial { peer, addr } => {
            swarm
                .behaviour_mut()
                .request_response
                .add_address(&peer, addr);
        }
        Event::Listen(addr) => {
            let addr = addr.with(Protocol::P2pCircuit);
            println!("{addr}");
            swarm.listen_on(addr).expect("listen failed");
        }

        Event::Join { peer, addr } => {
            swarm.behaviour_mut().kad.add_address(&peer, addr);
            // swarm.behaviour_mut().kad.bootstrap().expect("kad bootstrap failed");
        }
    }
}

#[derive(Debug)]
enum Event {
    Send { peer: PeerId, content: String },
    Dial { peer: PeerId, addr: Multiaddr },
    Listen(Multiaddr),
    Join { peer: PeerId, addr: Multiaddr },
}

#[derive(NetworkBehaviour)]
struct Behavior {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    request_response: request_response::Behaviour<message::Codec>,
    relay: relay_client::Behaviour,
    kad: Kademlia<kad::store::MemoryStore>,
}

#[cfg(test)]
mod tests {
    use libp2p::multiaddr::Protocol;
    use libp2p::{Multiaddr, PeerId};

    #[test]
    fn show_addr_with_relay_addr() {
        let addr =
            "/ip4/127.0.0.1/tcp/9000/p2p/12D3KooWHkc9bYV9vpxAXEuLXV6WGPAsEYH6oa9NCinsTRriBDzp"
                .parse::<Multiaddr>()
                .unwrap();

        let peer: PeerId = "12D3KooWJH767mJv8hYU7hMR5bGPq2gMSu3iHqtio1hqTamBnsw3"
            .parse()
            .unwrap();

        println!(
            "{}",
            addr.with(Protocol::P2pCircuit)
                .with(Protocol::P2p(peer.into()))
        );
    }

    #[test]
    fn show_addr() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000/p2p/12D3KooWHkc9bYV9vpxAXEuLXV6WGPAsEYH6oa9NCinsTRriBDzp/p2p-circuit/p2p/12D3KooWAXee94AC3otPyuVcpeMs3pyotknqN7eHaRNmo3D3tcY5"
            .parse().unwrap();

        println!("{addr}");
    }

    #[test]
    fn test_while_let() {
        fn ret_iter_i32() -> impl Iterator<Item = i32> {
            1..100
        }

        while let Some(x) = ret_iter_i32().next() {
            println!("{x}");
        }
    }
}
