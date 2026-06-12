use super::events::{
    AppServerEventState, AppServerNotification, handle_notification, parse_notification,
    send_finish_reason,
};
use super::options::CodexOptions;
use crate::{
    AppServerToolCall, AppServerToolResult, AppServerToolResultContent, AppServerToolRuntime,
    ChatRequest, ChatStream, FinishReason, StreamEvent,
};
use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, warn};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

mod command;
mod home;
mod params;
mod rpc;
mod session;
mod tools;

use home::IsolatedCodexHome;
pub(super) use params::{thread_start_params, turn_start_params};
use rpc::{JsonRpcConnection, RpcMessage};
pub(super) use tools::{dynamic_tools, parse_dynamic_tool_call, tool_response};

pub(super) struct AppServerSession {
    options: CodexOptions,
}

impl AppServerSession {
    pub(super) fn new(options: CodexOptions) -> Self {
        Self { options }
    }
}
