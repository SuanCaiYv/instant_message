use std::time::Duration;
use ahash::AHashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info, warn, error};
use crate::core::process;
use crate::entity::msg;
use crate::persistence::redis_ops;
use crate::util::base;

const BODY_BUF_LENGTH: usize = 1 << 16;
const MAX_FRIENDS_NUMBER: usize = 1 << 10;

pub type ConnectionMap = std::sync::Arc<tokio::sync::RwLock<AHashMap<u64, tokio::sync::mpsc::Sender<msg::Msg>>>>;
pub type StatusMap = std::sync::Arc<tokio::sync::RwLock<AHashMap<u64, u64>>>;
// todo 优化连接
pub type RedisOps = std::sync::Arc<tokio::sync::RwLock<redis_ops::RedisOps>>;

pub struct Server {
    address: String,
    connection_map: ConnectionMap,
    status_map: StatusMap,
    redis_ops: RedisOps,
}

impl Server {
    pub async fn new(address_server: String, address_redis: String) -> Self {
        let redis_ops = redis_ops::RedisOps::connect(address_redis).await;
        Self {
            address: address_server,
            connection_map: std::sync::Arc::new(tokio::sync::RwLock::new(AHashMap::new())),
            status_map: std::sync::Arc::new(tokio::sync::RwLock::new(AHashMap::new())),
            redis_ops: std::sync::Arc::new(tokio::sync::RwLock::new(redis_ops)),
        }
    }

    pub async fn run(self) {
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(self.address.clone()).await.unwrap();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                (&self).handle(stream).await;
            }
        });
    }

    async fn handle(&self, mut stream: tokio::net::TcpStream) {
        let mut c_map = self.connection_map.clone();
        let mut s_map = self.status_map.clone();
        let mut redis_ops = self.redis_ops.clone();
        tokio::spawn(async move {
            let mut head: Box<[u8; msg::HEAD_LEN]> = Box::new([0; msg::HEAD_LEN]);
            let mut body: Box<[u8; BODY_BUF_LENGTH]> = Box::new([0; BODY_BUF_LENGTH]);
            let mut head_buf = &mut (*head);
            let mut body_buf = &mut (*body);
            let mut socket = &mut stream;
            // 处理第一次读
            let (sender, mut receiver): (tokio::sync::mpsc::Sender<msg::Msg>, tokio::sync::mpsc::Receiver<msg::Msg>) = tokio::sync::mpsc::channel(10);
            if let Ok(msg) = Self::read_msg_from_stream(socket, head_buf, body_buf).await {
                {
                    let mut lock = c_map.write().await;
                    (*lock).insert(msg.head.sender, sender.clone());
                }
            }
            let mut c_map_ref = &mut c_map;
            let mut s_map_ref = &mut s_map;
            let mut redis_ops_ref = &mut redis_ops;
            loop {
                tokio::select! {
                    msg = Self::read_msg_from_stream(socket, head_buf, body_buf) => {
                        if let Ok(mut msg) = msg {
                            if let Ok(ref msg) = process::biz::process(&mut msg, c_map_ref, redis_ops_ref).await {
                                if let Err(e) = Self::write_msg_to_stream(socket, msg).await {
                                    error!("connection[{}] closed with: {}", socket.peer_addr().unwrap(), e);
                                    continue
                                }
                            }
                        } else {
                            error!("connection[{}] closed with: {}", socket.peer_addr().unwrap(), "read error");
                            break;
                        }
                    }
                    msg = receiver.recv() => {
                        if let Some(ref msg) = msg {
                            if let Err(e) = Self::write_msg_to_stream(socket, msg).await {
                                error!("connection[{}] closed with: {}", socket.peer_addr().unwrap(), e);
                                break;
                            }
                        } else {
                            error!("connection[{}] closed with: {}", socket.peer_addr().unwrap(), "receiver closed");
                            break;
                        }
                    }
                }
            }
        });
    }

    async fn read_msg_from_stream(stream: &mut tokio::net::TcpStream, head_buf: &mut [u8], body_buf: &mut [u8]) -> std::io::Result<msg::Msg> {
        if let Ok(readable_size) = stream.read(head_buf).await {
            if readable_size == 0 {
                warn!("connection:[{}] closed", stream.peer_addr().unwrap());
                stream.shutdown().await?;
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "connection closed"));
            }
            if readable_size != msg::HEAD_LEN {
                error!("read head error");
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "read head error"));
            }
            let mut head = msg::Head::from(&head_buf[..]);
            head.timestamp = base::timestamp();
            if let body_length = stream.read(&mut body_buf[0..head.length as usize]).await? {
                if body_length != head.length as usize {
                    error!("read body error");
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, "read body error"));
                }
            }
            let length = head.length;
            let msg = msg::Msg {
                head,
                payload: Vec::from(&body_buf[0..length as usize]),
            };
            Ok(msg)
        } else {
            error!("read head error");
            stream.shutdown().await?;
            Err(std::io::Error::new(std::io::ErrorKind::Other, "read head error"))
        }
    }

    async fn write_msg_to_stream(stream: &mut tokio::net::TcpStream, msg: &msg::Msg) -> std::io::Result<()> {
        stream.write(msg.as_bytes().as_slice()).await?;
        stream.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::core::net::{Server};

    #[tokio::test]
    async fn it_works() {
        Server::new("127.0.0.1:8190".to_string(), "127.0.0.1:6379".to_string()).await.run().await;
    }
}