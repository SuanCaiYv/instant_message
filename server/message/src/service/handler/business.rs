use std::sync::Arc;

use async_trait::async_trait;
use lib::{
    entity::Msg,
    net::server::{Handler, HandlerParameters, WrapMsgMpscSender},
    Result, error::HandlerError,
};
use anyhow::anyhow;
use tracing::{debug, error};
use lib::entity::Type;

use crate::{service::ClientConnectionMap, cluster::{ClusterConnectionMap}, util::my_id};

#[inline]
pub(self) async fn forward_only_user(msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
    let client_map = &parameters
        .generic_parameters
        .get_parameter::<ClientConnectionMap>()?
        .0;
    let cluster_map = &parameters
        .generic_parameters
        .get_parameter::<ClusterConnectionMap>()?
        .0;
    let io_task_sender = &parameters
        .generic_parameters
        .get_parameter::<WrapMsgMpscSender>()?
        .0;
    let receiver = msg.receiver();
    let node_id = msg.node_id();
    if node_id == my_id() {
        match client_map.get(&receiver) {
            Some(client_sender) => {
                client_sender.send(msg.clone()).await?;
            }
            None => {
                debug!("receiver {} not found", receiver);
            }
        }
        io_task_sender.send(msg.clone()).await?;
    } else {
        match cluster_map.get(&node_id) {
            Some(sender) => {
                sender.send(msg.clone()).await?;
            },
            None => {
                // todo cluster offline error handler.
                error!("cluster[{}] offline!", node_id);
            }
        }
    }
    Ok(msg.generate_ack(my_id()))
}

pub(crate) struct JoinGroup;

#[async_trait]
impl Handler for JoinGroup {
    async fn run(&self, msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::JoinGroup {
            return Err(anyhow!(HandlerError::NotMine));
        }
        forward_only_user(msg, parameters).await
    }
}

pub(crate) struct LeaveGroup;

#[async_trait]
impl Handler for LeaveGroup {
    async fn run(&self, msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::LeaveGroup {
            return Err(anyhow!(HandlerError::NotMine));
        }
        forward_only_user(msg, parameters).await
    }
}

pub(crate) struct AddFriend;

#[async_trait]
impl Handler for AddFriend {
    async fn run(&self, msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::AddFriend {
            return Err(anyhow!(HandlerError::NotMine));
        }
        forward_only_user(msg, parameters).await
    }
}

pub(crate) struct RemoveFriend;

#[async_trait]
impl Handler for RemoveFriend {
    async fn run(&self, msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::RemoveFriend {
            return Err(anyhow!(HandlerError::NotMine));
        }
        forward_only_user(msg, parameters).await
    }
}

pub(crate) struct SystemMessage;

#[async_trait]
impl Handler for SystemMessage {
    async fn run(&self, msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::SystemMessage {
            return Err(anyhow!(HandlerError::NotMine));
        }
        forward_only_user(msg, parameters).await
    }
}

pub(crate) struct RemoteInvoke;

#[async_trait]
impl Handler for RemoteInvoke {
    async fn run(&self, msg: Arc<Msg>, _parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::RemoteInvoke {
            return Err(anyhow!(HandlerError::NotMine));
        }
        Ok(msg.generate_ack(my_id()))
    }
}

pub(crate) struct SetRelationship;

#[async_trait]
impl Handler for SetRelationship {
    async fn run(&self, msg: Arc<Msg>, parameters: &mut HandlerParameters) -> Result<Msg> {
        if msg.typ() != Type::SetRelationship {
            return Err(anyhow!(HandlerError::NotMine));
        }
        forward_only_user(msg, parameters).await
    }
}
