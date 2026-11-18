//! Офлайн-аккаунты и Ely.by (authlib-injector).

use crate::net::{self, Result};
use crate::settings::Account;
use md5::{Digest, Md5};
use serde_json::json;

/// Совпадает с Java `UUID.nameUUIDFromBytes("OfflinePlayer:<name>")` — тот же UUID, что у ванильного офлайн-режима.
pub fn offline_uuid(name: &str) -> String {
    let mut b: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    b[6] = (b[6] & 0x0f) | 0x30;
    b[8] = (b[8] & 0x3f) | 0x80;
    hex::encode(b)
}

pub fn valid_nick(n: &str) -> bool {
    (3..=16).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn new_offline(name: &str) -> Result<Account> {
    if !valid_nick(name) { return Err("Ник: 3–16 символов, латиница, цифры и _".into()); }
    Ok(Account { id: format!("offline:{name}"), kind: "offline".into(), name: name.into(), uuid: offline_uuid(name), ..Default::default() })
}

pub const ELY_AUTH: &str = "https://authserver.ely.by/auth";

pub fn ely_login(agent: &ureq::Agent, base: &str, user: &str, password: &str) -> Result<Account> {
    let client_token = hex::encode(&rand_bytes());
    let resp = agent.post(&format!("{base}/authenticate"))
        .send_json(json!({"username": user, "password": password, "clientToken": client_token, "requestUser": true}));
    let v: serde_json::Value = match resp {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(_, r)) => {
            let m = r.into_json::<serde_json::Value>().ok().and_then(|j| j.get("errorMessage").and_then(|m| m.as_str().map(String::from)));
            return Err(m.unwrap_or_else(|| "Ely.by отклонил вход".into()));
        }
        Err(e) => return Err(format!("Ely.by недоступен: {e}")),
    };
    let p = v.get("selectedProfile").ok_or("в ответе Ely.by нет профиля (нужен аккаунт с ником)")?;
    let name = p.get("name").and_then(|n| n.as_str()).ok_or("нет имени профиля")?;
    Ok(Account {
        id: format!("ely:{name}"), kind: "ely".into(), name: name.into(),
        uuid: p.get("id").and_then(|n| n.as_str()).ok_or("нет id профиля")?.into(),
        access_token: v.get("accessToken").and_then(|t| t.as_str()).map(String::from),
        client_token: Some(client_token),
    })
}

fn rand_bytes() -> [u8; 16] { *uuid::Uuid::new_v4().as_bytes() }

/// Аргумент javaagent для authlib-injector (скины Ely.by в игре).
pub fn ely_agent_arg(jar: &std::path::Path) -> String { format!("-javaagent:{}=ely.by", jar.display()) }

pub fn authlib_injector_url(agent: &ureq::Agent) -> Result<String> {
    let j = net::get_json(agent, "https://authlib-injector.yushi.moe/artifact/latest.json")?;
    j.get("download_url").and_then(|u| u.as_str()).map(String::from).ok_or_else(|| "нет download_url у authlib-injector".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uuid_is_v3_and_stable() {
        let u = offline_uuid("Steve");
        assert_eq!(u.len(), 32);
        assert_eq!(&u[12..13], "3");
        assert!(matches!(&u[16..17], "8" | "9" | "a" | "b"));
        assert_eq!(u, offline_uuid("Steve"));
        assert_ne!(u, offline_uuid("Alex"));
    }
    #[test]
    fn nick_rules() {
        assert!(valid_nick("Steve_1"));
        assert!(!valid_nick("ab") && !valid_nick("a b c") && !valid_nick("Имя_кириллица") && !valid_nick(&"a".repeat(17)));
        assert!(new_offline("x").is_err());
        assert_eq!(new_offline("Steve").unwrap().kind, "offline");
    }
    #[test]
    fn ely_login_parses_and_reports_errors() {
        use std::collections::HashMap;
        let mut f = HashMap::new();
        f.insert("/auth/authenticate".to_string(), br#"{"accessToken":"AT","selectedProfile":{"id":"abc123","name":"Neo"}}"#.to_vec());
        // сервер-заглушка отвечает 200 на любой путь из карты (POST тоже)
        let base = net::testsrv::serve(f);
        let a = ely_login(&net::agent(), &format!("{base}/auth"), "u", "p").unwrap();
        assert_eq!((a.name.as_str(), a.access_token.as_deref(), a.uuid.as_str()), ("Neo", Some("AT"), "abc123"));
        assert!(ely_login(&net::agent(), &format!("{base}/none"), "u", "p").is_err());
    }
}
