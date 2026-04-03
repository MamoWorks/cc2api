use base64::Engine;
use once_cell::sync::Lazy;
use rand::Rng;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::model::account::{
    Account, BillingMode, CanonicalEnvData, CanonicalProcessData, CanonicalPromptEnvData,
};

/// header wire 大小写映射。
/// Go 的 HTTP 服务器规范化 header，此映射还原 Claude CLI 抓包原始大小写。
static HEADER_WIRE_CASING: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("accept", "Accept");
    m.insert("user-agent", "User-Agent");
    m.insert("x-stainless-retry-count", "X-Stainless-Retry-Count");
    m.insert("x-stainless-timeout", "X-Stainless-Timeout");
    m.insert("x-stainless-lang", "X-Stainless-Lang");
    m.insert("x-stainless-package-version", "X-Stainless-Package-Version");
    m.insert("x-stainless-os", "X-Stainless-OS");
    m.insert("x-stainless-arch", "X-Stainless-Arch");
    m.insert("x-stainless-runtime", "X-Stainless-Runtime");
    m.insert("x-stainless-runtime-version", "X-Stainless-Runtime-Version");
    m.insert("x-stainless-helper-method", "x-stainless-helper-method");
    m.insert(
        "anthropic-dangerous-direct-browser-access",
        "anthropic-dangerous-direct-browser-access",
    );
    m.insert("anthropic-version", "anthropic-version");
    m.insert("anthropic-beta", "anthropic-beta");
    m.insert("x-app", "x-app");
    m.insert("content-type", "content-type");
    m.insert("accept-language", "accept-language");
    m.insert("sec-fetch-mode", "sec-fetch-mode");
    m.insert("accept-encoding", "accept-encoding");
    m.insert("authorization", "authorization");
    m.insert("x-claude-code-session-id", "X-Claude-Code-Session-Id");
    m.insert("x-client-request-id", "x-client-request-id");
    m.insert("content-length", "content-length");
    m
});

/// 将规范化 key 转换为真实 wire 大小写。
fn resolve_wire_casing(key: &str) -> String {
    let lower = key.to_lowercase();
    if let Some(wk) = HEADER_WIRE_CASING.get(lower.as_str()) {
        wk.to_string()
    } else {
        key.to_string()
    }
}

/// 请求来源类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientType {
    ClaudeCode,
    API,
}

const DEFAULT_VERSION: &str = "2.1.81";

/// 合并必需的 beta 令牌与客户端传入的 beta 令牌。
fn merge_anthropic_beta(required: &str, incoming: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut tokens = Vec::new();
    for t in required.split(',') {
        let t = t.trim();
        if !t.is_empty() && seen.insert(t.to_string()) {
            tokens.push(t.to_string());
        }
    }
    for t in incoming.split(',') {
        let t = t.trim();
        if !t.is_empty() && seen.insert(t.to_string()) {
            tokens.push(t.to_string());
        }
    }
    tokens.join(",")
}

/// 根据模型返回正确的 anthropic-beta 值。
fn beta_header_for_model(model_id: &str) -> &'static str {
    let lower = model_id.to_lowercase();
    if lower.contains("haiku") {
        "oauth-2025-04-20,interleaved-thinking-2025-05-14"
    } else {
        "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14"
    }
}

/// 处理所有请求的反检测改写。
pub struct Rewriter;

impl Rewriter {
    pub fn new() -> Self {
        Self
    }

    // --- Header 改写 ---

    /// 处理出站 header 的反检测改写。
    pub fn rewrite_headers(
        &self,
        headers: &HashMap<String, String>,
        account: &Account,
        client_type: ClientType,
        model_id: &str,
        body_map: &serde_json::Value,
    ) -> HashMap<String, String> {
        let env = self.parse_env(account);
        let version = if env.version.is_empty() {
            DEFAULT_VERSION
        } else {
            &env.version
        };

        let mut out = HashMap::new();

        if client_type == ClientType::API {
            // API 模式：使用与真实 Claude CLI 匹配的固定 header 集合。
            out.insert("Accept".into(), "application/json".into());
            out.insert(
                "User-Agent".into(),
                format!("claude-code/{} (external, cli)", version),
            );
            out.insert(
                "anthropic-beta".into(),
                beta_header_for_model(model_id).into(),
            );
            out.insert("anthropic-version".into(), "2023-06-01".into());
            out.insert(
                "anthropic-dangerous-direct-browser-access".into(),
                "true".into(),
            );
            out.insert("x-app".into(), "cli".into());
            out.insert("content-type".into(), "application/json".into());
            out.insert(
                "accept-encoding".into(),
                "gzip, deflate, br, zstd".into(),
            );
            out.insert("X-Stainless-Lang".into(), "js".into());
            out.insert("X-Stainless-Package-Version".into(), "0.70.0".into());
            out.insert("X-Stainless-OS".into(), "Linux".into());
            out.insert("X-Stainless-Arch".into(), "arm64".into());
            out.insert("X-Stainless-Runtime".into(), "node".into());
            out.insert("X-Stainless-Runtime-Version".into(), "v24.13.0".into());
            out.insert("X-Stainless-Retry-Count".into(), "0".into());
            out.insert("X-Stainless-Timeout".into(), "600".into());

            let session_id = extract_session_id_from_body(body_map)
                .unwrap_or_else(generate_session_uuid);
            out.insert("X-Claude-Code-Session-Id".into(), session_id);
        } else {
            // CC 客户端模式：白名单 + 改写
            let allowed: std::collections::HashSet<&str> = [
                "accept",
                "user-agent",
                "content-type",
                "accept-encoding",
                "accept-language",
                "anthropic-beta",
                "anthropic-version",
                "anthropic-dangerous-direct-browser-access",
                "x-app",
                "sec-fetch-mode",
                "x-stainless-retry-count",
                "x-stainless-timeout",
                "x-stainless-lang",
                "x-stainless-package-version",
                "x-stainless-os",
                "x-stainless-arch",
                "x-stainless-runtime",
                "x-stainless-runtime-version",
                "x-stainless-helper-method",
                "x-claude-code-session-id",
                "x-client-request-id",
            ]
            .into_iter()
            .collect();

            for (k, v) in headers {
                let lower = k.to_lowercase();
                if !allowed.contains(lower.as_str()) {
                    continue;
                }
                let wire_key = resolve_wire_casing(k);
                match lower.as_str() {
                    "user-agent" => {
                        out.insert(
                            wire_key,
                            format!("claude-code/{} (external, cli)", version),
                        );
                    }
                    _ => {
                        out.insert(wire_key, v.clone());
                    }
                }
            }

            // 确保必需 header 存在
            out.entry("anthropic-dangerous-direct-browser-access".into())
                .or_insert_with(|| "true".into());

            // 合并客户端 beta 与必需 beta
            let existing_beta = out
                .get("anthropic-beta")
                .cloned()
                .unwrap_or_default();
            out.insert(
                "anthropic-beta".into(),
                merge_anthropic_beta(beta_header_for_model(model_id), &existing_beta),
            );
        }

        out
    }

    // --- Body 改写 ---

    /// 根据端点和客户端类型改写请求体。
    pub fn rewrite_body(
        &self,
        body: &[u8],
        path: &str,
        account: &Account,
        client_type: ClientType,
    ) -> Vec<u8> {
        if body.is_empty() {
            return body.to_vec();
        }

        let mut parsed: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return body.to_vec(), // 非 JSON，直接透传
        };

        if path.starts_with("/v1/messages") {
            strip_empty_text_blocks(&mut parsed);
            self.rewrite_messages(&mut parsed, account, client_type);
        } else if path.contains("/event_logging/batch") {
            self.rewrite_event_batch(&mut parsed, account);
        } else {
            self.rewrite_generic_identity(&mut parsed, account);
        }

        serde_json::to_vec(&parsed).unwrap_or_else(|_| body.to_vec())
    }

    /// 处理 /v1/messages 请求体。
    fn rewrite_messages(
        &self,
        body: &mut serde_json::Value,
        account: &Account,
        client_type: ClientType,
    ) {
        let env = self.parse_env(account);
        let prompt_env = self.parse_prompt_env(account);

        if client_type == ClientType::ClaudeCode {
            // 替换模式
            self.rewrite_metadata_user_id(body, account);
            self.rewrite_system_prompt(body, &prompt_env, &env.version, &account.billing_mode);
        } else {
            // 注入模式
            let session_id = self.inject_metadata_user_id(body, account);
            if let Some(sid) = &session_id {
                if let Some(metadata) = body.get_mut("metadata").and_then(|m| m.as_object_mut()) {
                    metadata.insert(
                        "_session_id".into(),
                        serde_json::Value::String(sid.clone()),
                    );
                }
            }

            // 剥离 Claude Code 不会发送的字段
            if let Some(obj) = body.as_object_mut() {
                obj.remove("temperature");
                obj.remove("top_k");
                obj.remove("top_p");
                obj.remove("stop_sequences");
                obj.remove("tool_choice");

                // 确保 tools 字段存在
                obj.entry("tools")
                    .or_insert(serde_json::Value::Array(vec![]));

                // 确保 stream 为 true
                obj.insert("stream".into(), serde_json::Value::Bool(true));
            }

            // 剥离 system 块中的 cache_control
            strip_cache_control(body);

            // 规范化 max_tokens
            if let Some(max_tokens) = body.get("max_tokens").and_then(|v| v.as_f64()) {
                if max_tokens > 32768.0 {
                    body.as_object_mut()
                        .unwrap()
                        .insert("max_tokens".into(), serde_json::json!(16384));
                }
            }

            // 注入 Claude Code 系统提示词
            self.inject_system_prompt(body);
        }
    }

    /// 替换已有 metadata.user_id 中的 device_id（CC 客户端模式）。
    fn rewrite_metadata_user_id(&self, body: &mut serde_json::Value, account: &Account) {
        let user_id_str = {
            let metadata = match body.get("metadata").and_then(|m| m.as_object()) {
                Some(m) => m,
                None => return,
            };
            match metadata.get("user_id").and_then(|u| u.as_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => return,
            }
        };

        // 尝试 JSON 格式
        if let Ok(mut uid) = serde_json::from_str::<serde_json::Value>(&user_id_str) {
            if let Some(obj) = uid.as_object_mut() {
                obj.insert(
                    "device_id".into(),
                    serde_json::Value::String(account.device_id.clone()),
                );
                let new_str = serde_json::to_string(&uid).unwrap_or_default();
                if let Some(metadata) =
                    body.get_mut("metadata").and_then(|m| m.as_object_mut())
                {
                    metadata.insert(
                        "user_id".into(),
                        serde_json::Value::String(new_str),
                    );
                }
                return;
            }
        }

        // 旧格式：user_{device}_account_{uuid}_session_{uuid}
        if let Some(idx) = user_id_str.find("_account_") {
            let new_val = format!(
                "user_{}_account_{}",
                account.device_id,
                &user_id_str[idx + 9..]
            );
            if let Some(metadata) = body.get_mut("metadata").and_then(|m| m.as_object_mut()) {
                metadata.insert("user_id".into(), serde_json::Value::String(new_val));
            }
        }
    }

    /// 为纯 API 调用创建 metadata.user_id。返回使用的 session_id。
    fn inject_metadata_user_id(
        &self,
        body: &mut serde_json::Value,
        account: &Account,
    ) -> Option<String> {
        // 确保 metadata 存在
        if body.get("metadata").is_none() {
            body.as_object_mut()
                .unwrap()
                .insert("metadata".into(), serde_json::json!({}));
        }

        // 已有 user_id，改为改写
        if body
            .get("metadata")
            .and_then(|m| m.get("user_id"))
            .is_some()
        {
            self.rewrite_metadata_user_id(body, account);
            return None;
        }

        let session_id = generate_session_uuid();
        let account_uuid = derive_account_uuid(account);
        let uid = serde_json::json!({
            "device_id": account.device_id,
            "account_uuid": account_uuid,
            "session_id": session_id,
        });
        let uid_str = serde_json::to_string(&uid).unwrap_or_default();
        if let Some(metadata) = body.get_mut("metadata").and_then(|m| m.as_object_mut()) {
            metadata.insert("user_id".into(), serde_json::Value::String(uid_str));
        }
        Some(session_id)
    }

    /// 将 Claude Code 系统提示词添加到请求体前面（仅 API 注入模式）。
    fn inject_system_prompt(&self, body: &mut serde_json::Value) {
        let banner_block = serde_json::json!({
            "type": "text",
            "text": CLAUDE_CODE_SYSTEM_PROMPT,
            "cache_control": { "type": "ephemeral" }
        });

        match body.get("system") {
            None => {
                body.as_object_mut().unwrap().insert(
                    "system".into(),
                    serde_json::Value::Array(vec![banner_block]),
                );
            }
            Some(serde_json::Value::String(sys)) => {
                if sys.starts_with(CLAUDE_CODE_SYSTEM_PROMPT) {
                    return;
                }
                let user_block = serde_json::json!({
                    "type": "text",
                    "text": sys,
                });
                body.as_object_mut().unwrap().insert(
                    "system".into(),
                    serde_json::Value::Array(vec![banner_block, user_block]),
                );
            }
            Some(serde_json::Value::Array(arr)) => {
                if let Some(first) = arr.first() {
                    if let Some(text) = first.get("text").and_then(|t| t.as_str()) {
                        if text.starts_with(CLAUDE_CODE_SYSTEM_PROMPT) {
                            return;
                        }
                    }
                }
                let mut new_arr = vec![banner_block];
                new_arr.extend(arr.iter().cloned());
                body.as_object_mut()
                    .unwrap()
                    .insert("system".into(), serde_json::Value::Array(new_arr));
            }
            _ => {}
        }
    }

    // --- 系统提示词改写（仅 CC 客户端模式）---

    fn rewrite_system_prompt(
        &self,
        body: &mut serde_json::Value,
        pe: &CanonicalPromptEnvData,
        version: &str,
        billing_mode: &BillingMode,
    ) {
        let version = if version.is_empty() {
            DEFAULT_VERSION
        } else {
            version
        };

        // CCH hash 计算
        let cch_hash = if *billing_mode == BillingMode::Rewrite {
            let first_msg = extract_first_user_message(body);
            if !first_msg.is_empty() {
                compute_cch(&first_msg, version)
            } else {
                let mut bytes = [0u8; 2];
                rand::thread_rng().fill(&mut bytes);
                format!("{:x}", u16::from_be_bytes(bytes))[..3].to_string()
            }
        } else {
            String::new()
        };

        let rewrite = |text: &str| -> String {
            let mut text = text.to_string();
            if *billing_mode == BillingMode::Rewrite {
                text = BILLING_REGEX
                    .replace_all(&text, &format!("cc_version={}.{}", version, cch_hash))
                    .to_string();
            } else {
                text = BILLING_LINE_REGEX.replace_all(&text, "").to_string();
                text = BILLING_REGEX.replace_all(&text, "").to_string();
            }
            text = PLATFORM_REGEX
                .replace_all(&text, &format!("Platform: {}", pe.platform))
                .to_string();
            text = SHELL_REGEX
                .replace_all(&text, &format!("Shell: {}", pe.shell))
                .to_string();
            text = OS_VERSION_REGEX
                .replace_all(&text, &format!("OS Version: {}", pe.os_version))
                .to_string();
            text = WORKING_DIR_REGEX
                .replace_all(&text, &format!("${{1}}{}", pe.working_dir))
                .to_string();
            let home_prefix = if let Some(idx) = nth_index(&pe.working_dir, '/', 3) {
                &pe.working_dir[..idx + 1]
            } else {
                &pe.working_dir
            };
            text = HOME_PATH_REGEX
                .replace_all(&text, home_prefix)
                .to_string();
            text
        };

        let rewrite_in_reminders = |text: &str| -> String {
            SYSTEM_REMINDER_REGEX
                .replace_all(text, |caps: &regex::Captures| rewrite(&caps[0]))
                .to_string()
        };

        // 改写 body.system
        match body.get("system").cloned() {
            Some(serde_json::Value::String(sys)) => {
                body.as_object_mut().unwrap().insert(
                    "system".into(),
                    serde_json::Value::String(rewrite(&sys)),
                );
            }
            Some(serde_json::Value::Array(sys)) => {
                let filtered: Vec<serde_json::Value> = if *billing_mode == BillingMode::Strip {
                    sys.iter()
                        .filter(|item| {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                if BILLING_LINE_REGEX.is_match(text) {
                                    let cleaned =
                                        BILLING_LINE_REGEX.replace_all(text, "").to_string();
                                    if cleaned.trim().is_empty() {
                                        return false;
                                    }
                                }
                            }
                            true
                        })
                        .cloned()
                        .collect()
                } else {
                    sys.clone()
                };

                let rewritten: Vec<serde_json::Value> = filtered
                    .into_iter()
                    .map(|mut item| {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            let new_text = rewrite(text);
                            item.as_object_mut()
                                .unwrap()
                                .insert("text".into(), serde_json::Value::String(new_text));
                        }
                        item
                    })
                    .collect();

                body.as_object_mut()
                    .unwrap()
                    .insert("system".into(), serde_json::Value::Array(rewritten));
            }
            _ => {}
        }

        // 改写消息 — 仅在 <system-reminder> 标签内替换
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            for msg in messages.iter_mut() {
                rewrite_message_content(msg, &rewrite_in_reminders);
            }
        }
    }

    // --- 事件日志批量改写 ---

    fn rewrite_event_batch(&self, body: &mut serde_json::Value, account: &Account) {
        let env = self.parse_env(account);
        let proc = self.parse_process(account);

        let events = match body.get_mut("events").and_then(|e| e.as_array_mut()) {
            Some(e) => e,
            None => return,
        };

        let canonical_env = build_canonical_env_map(&env);

        for event in events.iter_mut() {
            let e = match event.as_object_mut() {
                Some(e) => e,
                None => continue,
            };

            if e.contains_key("device_id") {
                e.insert(
                    "device_id".into(),
                    serde_json::Value::String(account.device_id.clone()),
                );
            }
            if e.contains_key("email") {
                e.insert(
                    "email".into(),
                    serde_json::Value::String(account.email.clone()),
                );
            }

            e.remove("baseUrl");
            e.remove("base_url");
            e.remove("gateway");

            if e.contains_key("env") {
                e.insert("env".into(), canonical_env.clone());
            }

            if let Some(p) = e.remove("process") {
                e.insert("process".into(), rewrite_process(&p, &proc));
            }

            if let Some(am) = e.get("additional_metadata").and_then(|v| v.as_str()) {
                let rewritten = rewrite_additional_metadata(am);
                e.insert(
                    "additional_metadata".into(),
                    serde_json::Value::String(rewritten),
                );
            }
        }
    }

    // --- 通用身份改写 ---

    fn rewrite_generic_identity(&self, body: &mut serde_json::Value, account: &Account) {
        if let Some(obj) = body.as_object_mut() {
            if obj.contains_key("device_id") {
                obj.insert(
                    "device_id".into(),
                    serde_json::Value::String(account.device_id.clone()),
                );
            }
            if obj.contains_key("email") {
                obj.insert(
                    "email".into(),
                    serde_json::Value::String(account.email.clone()),
                );
            }
        }
    }

    // --- 辅助解析 ---

    fn parse_env(&self, account: &Account) -> CanonicalEnvData {
        serde_json::from_value(account.canonical_env.clone()).unwrap_or_default()
    }

    fn parse_prompt_env(&self, account: &Account) -> CanonicalPromptEnvData {
        serde_json::from_value(account.canonical_prompt.clone()).unwrap_or_default()
    }

    fn parse_process(&self, account: &Account) -> CanonicalProcessData {
        serde_json::from_value(account.canonical_process.clone()).unwrap_or_default()
    }
}

// --- 正则表达式 ---

static PLATFORM_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Platform:\s*\S+").unwrap());
static SHELL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Shell:\s*\S+").unwrap());
static OS_VERSION_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"OS Version:\s*[^\n<]+").unwrap());
static WORKING_DIR_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"((?:Primary )?[Ww]orking directory:\s*)/\S+").unwrap());
static HOME_PATH_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"/(?:Users|home)/[^/\s]+/").unwrap());
static BILLING_LINE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*x-anthropic-billing-header:[^\n]*\n?").unwrap());
static BILLING_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"cc_version=[\d.]+\.[a-f0-9]{3};[^;]*;?").unwrap());
static SYSTEM_REMINDER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<system-reminder>(.*?)</system-reminder>").unwrap());

// --- CCH hash ---

const CCH_SALT: &str = "59cf53e54c78";
const CCH_POSITIONS: [usize; 3] = [4, 7, 20];

fn compute_cch(first_user_message_text: &str, version: &str) -> String {
    let bytes = first_user_message_text.as_bytes();
    let mut chars = Vec::new();
    for &pos in &CCH_POSITIONS {
        if pos < bytes.len() {
            chars.push(bytes[pos]);
        } else {
            chars.push(b'0');
        }
    }
    let input = format!("{}{}{}", CCH_SALT, String::from_utf8_lossy(&chars), version);
    let hash = Sha256::digest(input.as_bytes());
    format!("{:x}", hash)[..3].to_string()
}

/// 从 messages 数组中提取首条用户消息文本。
fn extract_first_user_message(body: &serde_json::Value) -> String {
    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(m) => m,
        None => return String::new(),
    };
    for msg in messages {
        let m = match msg.as_object() {
            Some(m) => m,
            None => continue,
        };
        if m.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        match m.get("content") {
            Some(serde_json::Value::String(c)) => return c.clone(),
            Some(serde_json::Value::Array(arr)) => {
                for item in arr {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        return text.to_string();
                    }
                }
            }
            _ => {}
        }
    }
    String::new()
}

fn rewrite_message_content<F>(msg: &mut serde_json::Value, rewrite_fn: &F)
where
    F: Fn(&str) -> String,
{
    match msg.get("content").cloned() {
        Some(serde_json::Value::String(s)) => {
            msg.as_object_mut().unwrap().insert(
                "content".into(),
                serde_json::Value::String(rewrite_fn(&s)),
            );
        }
        Some(serde_json::Value::Array(arr)) => {
            let rewritten: Vec<serde_json::Value> = arr
                .into_iter()
                .map(|mut item| {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        let new_text = rewrite_fn(text);
                        item.as_object_mut()
                            .unwrap()
                            .insert("text".into(), serde_json::Value::String(new_text));
                    }
                    item
                })
                .collect();
            msg.as_object_mut()
                .unwrap()
                .insert("content".into(), serde_json::Value::Array(rewritten));
        }
        _ => {}
    }
}

fn build_canonical_env_map(env: &CanonicalEnvData) -> serde_json::Value {
    serde_json::json!({
        "platform": env.platform,
        "platform_raw": env.platform_raw,
        "arch": env.arch,
        "node_version": env.node_version,
        "terminal": env.terminal,
        "package_managers": env.package_managers,
        "runtimes": env.runtimes,
        "is_running_with_bun": false,
        "is_ci": false,
        "is_claubbit": false,
        "is_claude_code_remote": false,
        "is_local_agent_mode": false,
        "is_conductor": false,
        "is_github_action": false,
        "is_claude_code_action": false,
        "is_claude_ai_auth": env.is_claude_ai_auth,
        "version": env.version,
        "version_base": env.version_base,
        "build_time": env.build_time,
        "deployment_environment": env.deployment_environment,
        "vcs": env.vcs,
    })
}

// --- 进程指纹改写 ---

fn rewrite_process(original: &serde_json::Value, proc: &CanonicalProcessData) -> serde_json::Value {
    let engine = base64::engine::general_purpose::STANDARD;
    match original {
        serde_json::Value::String(s) => {
            let decoded = match engine.decode(s) {
                Ok(d) => d,
                Err(_) => return original.clone(),
            };
            let mut obj: serde_json::Value = match serde_json::from_slice(&decoded) {
                Ok(v) => v,
                Err(_) => return original.clone(),
            };
            rewrite_process_fields(&mut obj, proc);
            let out = serde_json::to_vec(&obj).unwrap_or_default();
            serde_json::Value::String(engine.encode(&out))
        }
        serde_json::Value::Object(_) => {
            let mut obj = original.clone();
            rewrite_process_fields(&mut obj, proc);
            obj
        }
        _ => original.clone(),
    }
}

fn rewrite_process_fields(obj: &mut serde_json::Value, proc: &CanonicalProcessData) {
    if let Some(map) = obj.as_object_mut() {
        map.insert(
            "constrainedMemory".into(),
            serde_json::json!(proc.constrained_memory),
        );
        map.insert(
            "rss".into(),
            serde_json::json!(random_in_range(proc.rss_range[0], proc.rss_range[1])),
        );
        map.insert(
            "heapTotal".into(),
            serde_json::json!(random_in_range(
                proc.heap_total_range[0],
                proc.heap_total_range[1]
            )),
        );
        map.insert(
            "heapUsed".into(),
            serde_json::json!(random_in_range(
                proc.heap_used_range[0],
                proc.heap_used_range[1]
            )),
        );
    }
}

// --- Base64 additional_metadata 改写 ---

fn rewrite_additional_metadata(encoded: &str) -> String {
    let engine = base64::engine::general_purpose::STANDARD;
    let decoded = match engine.decode(encoded) {
        Ok(d) => d,
        Err(_) => return encoded.to_string(),
    };
    let mut obj: serde_json::Value = match serde_json::from_slice(&decoded) {
        Ok(v) => v,
        Err(_) => return encoded.to_string(),
    };
    if let Some(map) = obj.as_object_mut() {
        map.remove("baseUrl");
        map.remove("base_url");
        map.remove("gateway");
    }
    let out = serde_json::to_vec(&obj).unwrap_or_default();
    engine.encode(&out)
}

/// 移除 system 和消息内容块中的 cache_control。
fn strip_cache_control(body: &mut serde_json::Value) {
    if let Some(sys) = body.get_mut("system").and_then(|s| s.as_array_mut()) {
        for item in sys.iter_mut() {
            if let Some(block) = item.as_object_mut() {
                block.remove("cache_control");
            }
        }
    }
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                for item in content.iter_mut() {
                    if let Some(block) = item.as_object_mut() {
                        block.remove("cache_control");
                    }
                }
            }
        }
    }
}

/// 移除消息和 system 中的空文本内容块。
fn strip_empty_text_blocks(body: &mut serde_json::Value) {
    fn filter_blocks(blocks: &mut Vec<serde_json::Value>) {
        blocks.retain(|item| {
            if let Some(block) = item.as_object() {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if text.is_empty() {
                        return false;
                    }
                }
            }
            true
        });
        // Handle tool_result nested content
        for item in blocks.iter_mut() {
            if let Some(block) = item.as_object_mut() {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    if let Some(content) = block.get_mut("content").and_then(|c| c.as_array_mut())
                    {
                        filter_blocks(content);
                    }
                }
            }
        }
    }

    if let Some(sys) = body.get_mut("system").and_then(|s| s.as_array_mut()) {
        filter_blocks(sys);
    }
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                filter_blocks(content);
            }
        }
    }
}

/// 从注入模式 body 中获取暂存的 _session_id。
pub fn extract_session_id_from_body(body: &serde_json::Value) -> Option<String> {
    body.get("metadata")
        .and_then(|m| m.get("_session_id"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// 清理 body 中的内部 _session_id 标记。
pub fn clean_session_id_from_body(body: &mut serde_json::Value) {
    if let Some(metadata) = body.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        metadata.remove("_session_id");
    }
}

/// 判断请求来自 Claude Code 还是纯 API。
pub fn detect_client_type(user_agent: &str, body: &serde_json::Value) -> ClientType {
    if user_agent.to_lowercase().starts_with("claude-cli/") {
        return ClientType::ClaudeCode;
    }
    if let Some(metadata) = body.get("metadata").and_then(|m| m.as_object()) {
        if metadata.contains_key("user_id") {
            return ClientType::ClaudeCode;
        }
    }
    ClientType::API
}

const CLAUDE_CODE_SYSTEM_PROMPT: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// 通过账号信息生成稳定的 UUID 标识符。
fn derive_account_uuid(account: &Account) -> String {
    let seed = if account.email.is_empty() {
        format!("account-{}", account.id)
    } else {
        account.email.clone()
    };
    let hash = Sha256::digest(seed.as_bytes());
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&hash[0..4]),
        hex::encode(&hash[4..6]),
        hex::encode(&hash[6..8]),
        hex::encode(&hash[8..10]),
        hex::encode(&hash[10..16])
    )
}

pub fn generate_session_uuid() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&b[0..4]),
        hex::encode(&b[4..6]),
        hex::encode(&b[6..8]),
        hex::encode(&b[8..10]),
        hex::encode(&b[10..16])
    )
}

fn random_in_range(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    rand::thread_rng().gen_range(min..max)
}

fn nth_index(s: &str, c: char, n: usize) -> Option<usize> {
    let mut count = 0;
    for (i, ch) in s.chars().enumerate() {
        if ch == c {
            count += 1;
            if count == n {
                return Some(i);
            }
        }
    }
    None
}
