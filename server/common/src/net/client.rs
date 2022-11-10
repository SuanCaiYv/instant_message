use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use super::MsgIOTimeOut;
use crate::entity::Msg;
use crate::net::LenBuffer;
use crate::net::MsgIO;
use crate::net::{InnerReceiver, InnerSender, OuterReceiver, OuterSender, ALPN_PRIM};
use crate::util::default_bind_ip;
use crate::Result;
use anyhow::anyhow;
use quinn::{Connection, Endpoint, StreamId, VarInt};
use tokio::select;

#[allow(unused)]
#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub remote_address: SocketAddr,
    pub domain: String,
    pub cert: rustls::Certificate,
    /// should set only on clients.
    pub keep_alive_interval: Duration,
    pub max_bi_streams: VarInt,
    pub max_uni_streams: VarInt,
    pub max_sender_side_channel_size: usize,
    pub max_receiver_side_channel_size: usize,
}

pub struct ClientConfigBuilder {
    #[allow(unused)]
    pub remote_address: Option<SocketAddr>,
    #[allow(unused)]
    pub domain: Option<String>,
    #[allow(unused)]
    pub cert: Option<rustls::Certificate>,
    #[allow(unused)]
    pub keep_alive_interval: Option<Duration>,
    #[allow(unused)]
    pub max_bi_streams: Option<VarInt>,
    #[allow(unused)]
    pub max_uni_streams: Option<VarInt>,
    #[allow(unused)]
    pub max_sender_side_channel_size: Option<usize>,
    #[allow(unused)]
    pub max_receiver_side_channel_size: Option<usize>,
}

impl Default for ClientConfigBuilder {
    fn default() -> Self {
        Self {
            remote_address: None,
            domain: None,
            cert: None,
            keep_alive_interval: None,
            max_bi_streams: None,
            max_uni_streams: None,
            max_sender_side_channel_size: None,
            max_receiver_side_channel_size: None,
        }
    }
}

impl ClientConfigBuilder {
    pub fn with_remote_address(&mut self, remote_address: SocketAddr) -> &mut Self {
        self.remote_address = Some(remote_address);
        self
    }

    pub fn with_domain(&mut self, domain: String) -> &mut Self {
        self.domain = Some(domain);
        self
    }

    pub fn with_cert(&mut self, cert: rustls::Certificate) -> &mut Self {
        self.cert = Some(cert);
        self
    }

    pub fn with_keep_alive_interval(&mut self, keep_alive_interval: Duration) -> &mut Self {
        self.keep_alive_interval = Some(keep_alive_interval);
        self
    }

    pub fn with_max_bi_streams(&mut self, max_bi_streams: VarInt) -> &mut Self {
        self.max_bi_streams = Some(max_bi_streams);
        self
    }

    pub fn with_max_uni_streams(&mut self, max_uni_streams: VarInt) -> &mut Self {
        self.max_uni_streams = Some(max_uni_streams);
        self
    }

    pub fn with_max_sender_side_channel_size(&mut self, max_sender_side_channel_size: usize) -> &mut Self {
        self.max_sender_side_channel_size = Some(max_sender_side_channel_size);
        self
    }

    pub fn with_max_receiver_side_channel_size(&mut self, max_receiver_side_channel_size: usize) -> &mut Self {
        self.max_receiver_side_channel_size = Some(max_receiver_side_channel_size);
        self
    }

    pub fn build(self) -> Result<ClientConfig> {
        let remote_address = self
            .remote_address
            .ok_or_else(|| anyhow!("address is required"))?;
        let domain = self.domain.ok_or_else(|| anyhow!("domain is required"))?;
        let cert = self.cert.ok_or_else(|| anyhow!("cert is required"))?;
        let keep_alive_interval = self
            .keep_alive_interval
            .ok_or_else(|| anyhow!("keep_alive_interval is required"))?;
        let max_bi_streams = self
            .max_bi_streams
            .ok_or_else(|| anyhow!("max_bi_streams is required"))?;
        let max_uni_streams = self
            .max_uni_streams
            .ok_or_else(|| anyhow!("max_uni_streams is required"))?;
        let max_sender_side_channel_size = self
            .max_sender_side_channel_size
            .ok_or_else(|| anyhow!("max_io_channel_size is required"))?;
        let max_receiver_side_channel_size = self
            .max_receiver_side_channel_size
            .ok_or_else(|| anyhow!("max_task_channel_size is required"))?;
        Ok(ClientConfig {
            remote_address,
            domain,
            cert,
            keep_alive_interval,
            max_bi_streams,
            max_uni_streams,
            max_sender_side_channel_size,
            max_receiver_side_channel_size,
        })
    }
}

/// the client is multi-stream designed.
/// That means the minimum unit to handle is the [`quinn::SendStream`] and [`quinn::RecvStream`]
pub struct Client {
    config: Option<ClientConfig>,
    endpoint: Option<Endpoint>,
    connection: Option<Connection>,
    outer_streams: Option<(OuterSender, OuterReceiver)>,
    inner_streams: Option<(InnerSender, InnerReceiver)>,
}

impl Client {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config: Some(config),
            endpoint: None,
            connection: None,
            outer_streams: None,
            inner_streams: None,
        }
    }

    /// quic allows to make more than one connections to the **same** server.
    /// but in fact, with the same server we only want one connection.
    /// so we choose to disable this ability, and for concurrent requests, just by multi-streams.
    pub async fn run(&mut self) -> Result<()> {
        let ClientConfig {
            remote_address,
            domain,
            cert,
            keep_alive_interval,
            max_bi_streams,
            max_uni_streams,
            max_sender_side_channel_size,
            max_receiver_side_channel_size,
        } = self.config.take().unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(&cert)?;
        let mut client_crypto = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_crypto.alpn_protocols = ALPN_PRIM.iter().map(|&x| x.into()).collect();
        let mut endpoint = quinn::Endpoint::client(default_bind_ip())?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(client_crypto));
        Arc::get_mut(&mut client_config.transport)
            .unwrap()
            .max_concurrent_bidi_streams(max_bi_streams)
            .max_concurrent_uni_streams(max_uni_streams)
            .keep_alive_interval(Some(keep_alive_interval));
        endpoint.set_default_client_config(client_config);
        let new_connection = endpoint
            .connect(remote_address, domain.as_str())
            .unwrap()
            .await
            .map_err(|e| anyhow!("failed to connect: {:?}", e))?;
        let quinn::NewConnection { connection, .. } = new_connection;
        let (inner_sender, outer_receiver) = tokio::sync::mpsc::channel(max_receiver_side_channel_size);
        let (outer_sender, inner_receiver) = async_channel::bounded(max_sender_side_channel_size);
        self.endpoint = Some(endpoint);
        self.connection = Some(connection);
        self.outer_streams = Some((outer_sender, outer_receiver));
        self.inner_streams = Some((inner_sender, inner_receiver));
        Ok(())
    }

    #[allow(unused)]
    pub async fn new_net_streams(&mut self) -> Result<StreamId> {
        let mut streams = self.connection.as_ref().unwrap().open_bi().await?;
        let stream_id = streams.0.id();
        let inner_streams = self.inner_streams.as_ref().unwrap();
        let inner_streams = (inner_streams.0.clone(), inner_streams.1.clone());
        let id = streams.0.id();
        tokio::spawn(async move {
            let mut buffer: Box<LenBuffer> = Box::new([0; 4]);
            loop {
                select! {
                    msg = MsgIO::read_msg(&mut buffer, &mut streams.1) => {
                        if let Ok(msg) = msg {
                            let res = inner_streams.0.send(msg).await;
                            if res.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    },
                    msg = inner_streams.1.recv() => {
                        if let Ok(msg) = msg {
                            let res = MsgIO::write_msg(msg, &mut streams.0).await;
                            if res.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        });
        Ok(stream_id)
    }

    #[allow(unused)]
    pub async fn finished_streams(&mut self, stream_id: StreamId) -> Result<()> {
        todo!()
    }

    #[allow(unused)]
    pub async fn rw_streams(
        &mut self,
        user_id: u64,
        node_id: u32,
        token: String,
    ) -> Result<(OuterSender, OuterReceiver)> {
        self.new_net_streams().await?;
        let mut streams = self.outer_streams.take().unwrap();
        let auth = Msg::auth(user_id, 0, node_id, token);
        streams.0.send(Arc::new(auth)).await?;
        let msg = streams.1.recv().await;
        if msg.is_none() {
            Err(anyhow!("auth failed"))
        } else {
            Ok(streams)
        }
    }

    #[allow(unused)]
    pub async fn rw_streams_no_token(&mut self) -> Result<(OuterSender, OuterReceiver)> {
        self.new_net_streams().await?;
        let mut streams = self.outer_streams.take().unwrap();
        Ok(streams)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(endpoint) = self.endpoint.as_ref() {
            endpoint.close(0u32.into(), b"it's time to say goodbye.");
        }
    }
}

pub struct ClientTimeout {
    config: Option<ClientConfig>,
    endpoint: Option<Endpoint>,
    connection: Option<Connection>,
    /// providing operations for outer caller to interact with the underlayer io.
    outer_channel: Option<(OuterSender, OuterReceiver)>,
    inner_channel: Option<(InnerSender, InnerReceiver)>,
    timeout_channel_sender: Option<InnerSender>,
    timeout_channel_receiver: Option<OuterReceiver>,
    timeout: Duration,
}

impl ClientTimeout {
    pub fn new(config: ClientConfig, timeout: Duration) -> Self {
        Self {
            config: Some(config),
            endpoint: None,
            connection: None,
            outer_channel: None,
            inner_channel: None,
            timeout_channel_sender: None,
            timeout_channel_receiver: None,
            timeout,
        }
    }

    /// quic allows to make more than one connections to the **same** server.
    /// but in fact, with the same server we only want one connection.
    /// so we choose to disable this ability, and for concurrent requests, just by multi-streams.
    pub async fn run(&mut self) -> Result<()> {
        let ClientConfig {
            remote_address,
            domain,
            cert,
            keep_alive_interval,
            max_bi_streams,
            max_uni_streams,
            max_sender_side_channel_size,
            max_receiver_side_channel_size,
        } = self.config.take().unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(&cert)?;
        let mut client_crypto = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_crypto.alpn_protocols = ALPN_PRIM.iter().map(|&x| x.into()).collect();
        let mut endpoint = quinn::Endpoint::client(default_bind_ip())?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(client_crypto));
        Arc::get_mut(&mut client_config.transport)
            .unwrap()
            .max_concurrent_bidi_streams(max_bi_streams)
            .max_concurrent_uni_streams(max_uni_streams)
            .keep_alive_interval(Some(keep_alive_interval));
        endpoint.set_default_client_config(client_config);
        let new_connection = endpoint
            .connect(remote_address, domain.as_str())
            .unwrap()
            .await
            .map_err(|e| anyhow!("failed to connect: {:?}", e))?;
        let quinn::NewConnection { connection, .. } = new_connection;
        let (inner_sender, outer_receiver) = tokio::sync::mpsc::channel(max_receiver_side_channel_size);
        let (outer_sender, inner_receiver) = async_channel::bounded(max_sender_side_channel_size);
        let (timeout_sender, timeout_receiver) = tokio::sync::mpsc::channel(max_receiver_side_channel_size);
        self.endpoint = Some(endpoint);
        self.connection = Some(connection);
        self.outer_channel = Some((outer_sender, outer_receiver));
        self.inner_channel = Some((inner_sender, inner_receiver));
        self.timeout_channel_sender = Some(timeout_sender);
        self.timeout_channel_receiver = Some(timeout_receiver);
        Ok(())
    }

    #[allow(unused)]
    pub async fn new_net_streams(&mut self) -> Result<StreamId> {
        let mut io_streams = self.connection.as_ref().unwrap().open_bi().await?;
        let stream_id = io_streams.0.id();
        let inner = self.inner_channel.as_ref().unwrap();
        let (inner_sender, inner_receiver) = (inner.0.clone(), inner.1.clone());
        let timeout_channel_sender = self.timeout_channel_sender.as_ref().unwrap().clone();
        let id = io_streams.0.id();
        let mut msg_io_timeout = MsgIOTimeOut::new(io_streams, self.timeout);
        let (mut recv_channel, send_channel, mut timeout_channel_receiver) =
            msg_io_timeout.channels();
        tokio::spawn(async move {
            loop {
                select! {
                    msg = recv_channel.recv() => {
                        if let Some(msg) = msg {
                            let res = inner_sender.send(msg).await;
                            if res.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    },
                    msg = inner_receiver.recv() => {
                        if let Ok(msg) = msg {
                            let res = send_channel.send(msg).await;;
                            if res.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    },
                    msg = timeout_channel_receiver.recv() => {
                        if let Some(msg) = msg {
                            let res = timeout_channel_sender.send(msg).await;
                            if res.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    },
                }
            }
        });
        Ok(stream_id)
    }

    #[allow(unused)]
    pub async fn finished_streams(&mut self, stream_id: StreamId) -> Result<()> {
        todo!()
    }

    #[allow(unused)]
    pub async fn rw_streams(
        &mut self,
        user_id: u64,
        node_id: u32,
        token: String,
    ) -> Result<(OuterSender, OuterReceiver, OuterReceiver)> {
        self.new_net_streams().await?;
        let (outer_sender, mut outer_receiver) = self.outer_channel.take().unwrap();
        let timeout_channel_receiver = self.timeout_channel_receiver.take().unwrap();
        let auth = Msg::auth(user_id, 0, node_id, token);
        outer_sender.send(Arc::new(auth)).await?;
        let msg = outer_receiver.recv().await;
        if msg.is_none() {
            Err(anyhow!("auth failed"))
        } else {
            Ok((outer_sender, outer_receiver, timeout_channel_receiver))
        }
    }
}

impl Drop for ClientTimeout {
    fn drop(&mut self) {
        if let Some(endpoint) = self.endpoint.as_ref() {
            endpoint.close(0u32.into(), b"it's time to say goodbye.");
        }
    }
}

/// this client can connect to multi server with same local address.
/// and this ability is provided by udp's bigram feature.
/// so, we don't need to use multi client to connect to multi server.
#[derive(Debug)]
pub struct ClientMultiConnection {
    endpoint: Endpoint,
    max_sender_side_channel_size: usize,
    max_receiver_side_channel_size: usize,
}

#[derive(Debug)]
pub struct ClientSubConnectionConfig {
    pub remote_address: SocketAddr,
    pub domain: String,
    pub opened_bi_streams_number: usize,
    pub opened_uni_streams_number: usize,
}

#[derive(Debug)]
pub struct ClientSubConnection {
    outer_channel: Option<(OuterSender, OuterReceiver)>,
}

impl ClientMultiConnection {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let ClientConfig {
            cert,
            keep_alive_interval,
            max_bi_streams,
            max_uni_streams,
            max_sender_side_channel_size,
            max_receiver_side_channel_size,
            ..
        } = config;
        let mut roots = rustls::RootCertStore::empty();
        roots.add(&cert)?;
        let mut client_crypto = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_crypto.alpn_protocols = ALPN_PRIM.iter().map(|&x| x.into()).collect();
        let mut endpoint = quinn::Endpoint::client(default_bind_ip())?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(client_crypto));
        Arc::get_mut(&mut client_config.transport)
            .unwrap()
            .max_concurrent_bidi_streams(max_bi_streams)
            .max_concurrent_uni_streams(max_uni_streams)
            .keep_alive_interval(Some(keep_alive_interval));
        endpoint.set_default_client_config(client_config);
        Ok(Self {
            endpoint,
            max_sender_side_channel_size,
            max_receiver_side_channel_size,
        })
    }

    pub async fn new_connection(
        &self,
        config: ClientSubConnectionConfig,
    ) -> Result<ClientSubConnection> {
        let ClientSubConnectionConfig {
            remote_address,
            domain,
            opened_bi_streams_number: opend_bi_streams_number,
            ..
        } = config;
        let new_connection = self.endpoint
            .connect(remote_address, domain.as_str())
            .unwrap()
            .await
            .map_err(|e| anyhow!("failed to connect: {:?}", e))?;
        let quinn::NewConnection { connection, .. } = new_connection;
        let (inner_sender, outer_receiver) = tokio::sync::mpsc::channel(self.max_receiver_side_channel_size);
        let (outer_sender, inner_receiver) = async_channel::bounded(self.max_sender_side_channel_size);
        for _ in 0..opend_bi_streams_number {
            let mut io_streams = connection.open_bi().await?;
            let inner_channel = (inner_sender.clone(), inner_receiver.clone());
            tokio::spawn(async move {
                let mut buffer: Box<LenBuffer> = Box::new([0; 4]);
                loop {
                    select! {
                        msg = MsgIO::read_msg(&mut buffer, &mut io_streams.1) => {
                            if let Ok(msg) = msg {
                                let res = inner_channel.0.send(msg).await;
                                if res.is_err() {
                                    break;
                                }
                            } else {
                                break;
                            }
                        },
                        msg = inner_channel.1.recv() => {
                            if let Ok(msg) = msg {
                                let res = MsgIO::write_msg(msg, &mut io_streams.0).await;
                                if res.is_err() {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            });
        }
        // we not implement uni stream
        Ok(ClientSubConnection {
            outer_channel: Some((outer_sender, outer_receiver)),
        })
    }
}

impl Drop for ClientMultiConnection {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"it's time to say goodbye.");
    }
}

impl ClientSubConnection {
    pub async fn operation_channel(
        &mut self,
    ) -> Result<(OuterSender, OuterReceiver)> {
        let (outer_sender, outer_receiver) = self.outer_channel.take().unwrap();
        Ok((outer_sender, outer_receiver))
    }
}
