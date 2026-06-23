use axum::http::{HeaderMap, StatusCode};

const ALL_SCOPES: [&str; 17] = [
    "run:write",
    "run:read",
    "context:write",
    "context:read",
    "coordination:read",
    "graph:read",
    "graph:write",
    "federation:write",
    "admin:read",
    "rustyred_thg:graph:read",
    "rustyred_thg:graph:query",
    "rustyred_thg:graph:context",
    "rustyred_thg:graph:write:propose",
    "rustyred_thg:graph:write:apply",
    "rustyred_thg:graph:index:read",
    "rustyred_thg:graph:admin:verify",
    "rustyred_thg:events:read",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiToken {
    pub token: String,
    pub tenant_id: String,
    pub scopes: Vec<String>,
}

impl ApiToken {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let (token_part, scopes_part) = trimmed
            .split_once('=')
            .or_else(|| trimmed.split_once(':'))?;
        let (token, tenant_from_token) = split_token_tenant(token_part);
        let mut tenant_id = tenant_from_token;
        let mut scopes = Vec::new();
        for scope in scopes_part
            .split(['|', ' ', '+'])
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
        {
            if let Some(scope_tenant) = scope
                .strip_prefix("tenant:")
                .or_else(|| scope.strip_prefix("tenant="))
            {
                let scope_tenant = scope_tenant.trim();
                if scope_tenant.is_empty() {
                    return None;
                }
                match &tenant_id {
                    Some(existing) if existing != scope_tenant => return None,
                    Some(_) => {}
                    None => tenant_id = Some(scope_tenant.to_string()),
                }
            } else if scope == "*" {
                return None;
            } else {
                scopes.push(scope.to_string());
            }
        }

        let token = token.trim();
        let tenant_id = tenant_id?;
        if token.is_empty() || tenant_id.trim().is_empty() || scopes.is_empty() {
            return None;
        }

        Some(Self {
            token: token.to_string(),
            tenant_id,
            scopes,
        })
    }

    #[cfg(test)]
    fn allows(&self, required_scope: &str) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope == required_scope || scope_alias(scope) == required_scope)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthContext {
    pub token: String,
    pub tenant_id: Option<String>,
    pub scopes: Vec<String>,
}

impl AuthContext {
    pub fn require_tenant(&self, tenant_id: &str) -> Result<(), StatusCode> {
        match self.tenant_id.as_deref() {
            Some(bound) if bound == tenant_id => Ok(()),
            Some(_) => Err(StatusCode::FORBIDDEN),
            None => Ok(()),
        }
    }
}

pub fn authenticate(
    headers: &HeaderMap,
    valid_tokens: &[ApiToken],
    require_auth: bool,
) -> Result<AuthContext, StatusCode> {
    if !require_auth {
        return Ok(AuthContext {
            token: "dev".to_string(),
            tenant_id: None,
            scopes: ALL_SCOPES
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
        });
    }

    let header = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_string();

    let matched = valid_tokens
        .iter()
        .find(|candidate| candidate.token == token)
        .ok_or(StatusCode::FORBIDDEN)?;

    Ok(AuthContext {
        token,
        tenant_id: Some(matched.tenant_id.clone()),
        scopes: matched.scopes.clone(),
    })
}

pub fn require_scope(
    headers: &HeaderMap,
    valid_tokens: &[ApiToken],
    required_scope: &str,
    require_auth: bool,
) -> Result<AuthContext, StatusCode> {
    let context = authenticate(headers, valid_tokens, require_auth)?;
    if !context
        .scopes
        .iter()
        .any(|scope| scope == required_scope || scope_alias(scope) == required_scope)
    {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(context)
}

pub fn require_scope_for_tenant(
    headers: &HeaderMap,
    valid_tokens: &[ApiToken],
    required_scope: &str,
    require_auth: bool,
    tenant_id: &str,
) -> Result<AuthContext, StatusCode> {
    let context = require_scope(headers, valid_tokens, required_scope, require_auth)?;
    context.require_tenant(tenant_id)?;
    Ok(context)
}

fn scope_alias(scope: &str) -> &str {
    match scope {
        "rustyred_thg:graph:read"
        | "rustyred_thg:graph:query"
        | "rustyred_thg:graph:index:read" => "graph:read",
        "rustyred_thg:graph:write:propose" | "rustyred_thg:graph:write:apply" => "graph:write",
        "rustyred_thg:graph:context" => "context:read",
        "rustyred_thg:graph:admin:verify" => "admin:read",
        other => other,
    }
}

fn split_token_tenant(raw: &str) -> (&str, Option<String>) {
    let trimmed = raw.trim();
    match trimmed.rsplit_once('@') {
        Some((token, tenant)) if !token.trim().is_empty() && !tenant.trim().is_empty() => {
            (token.trim(), Some(tenant.trim().to_string()))
        }
        _ => (trimmed, None),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, StatusCode};

    use super::{require_scope, require_scope_for_tenant, ApiToken};

    #[test]
    fn rejects_missing_bearer_token_when_auth_required() {
        let headers = HeaderMap::new();
        let tokens = vec![ApiToken {
            token: "secret".to_string(),
            tenant_id: "tenant-a".to_string(),
            scopes: vec!["run:read".to_string()],
        }];
        let result = require_scope(&headers, &tokens, "run:read", true);

        assert_eq!(result.unwrap_err(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn accepts_matching_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        let tokens = vec![ApiToken {
            token: "secret".to_string(),
            tenant_id: "tenant-a".to_string(),
            scopes: vec!["run:read".to_string()],
        }];

        let result = require_scope(&headers, &tokens, "run:read", true).unwrap();

        assert_eq!(result.token, "secret");
        assert_eq!(result.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(result.scopes, vec!["run:read"]);
    }

    #[test]
    fn rejects_token_without_required_scope() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        let tokens = vec![ApiToken {
            token: "secret".to_string(),
            tenant_id: "tenant-a".to_string(),
            scopes: vec!["run:read".to_string()],
        }];

        let result = require_scope(&headers, &tokens, "run:write", true);

        assert_eq!(result.unwrap_err(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn parses_scoped_token_from_env_value() {
        let token = ApiToken::parse("secret@tenant-a=run:read|graph:read").unwrap();

        assert_eq!(token.token, "secret");
        assert_eq!(token.tenant_id, "tenant-a");
        assert!(token.allows("run:read"));
        assert!(token.allows("graph:read"));
        assert!(!token.allows("admin:read"));
    }

    #[test]
    fn accepts_thg_scope_aliases_for_mcp_tokens() {
        let token = ApiToken::parse(
            "secret=tenant:tenant-a|rustyred_thg:graph:read|rustyred_thg:graph:admin:verify",
        )
        .unwrap();

        assert!(token.allows("graph:read"));
        assert!(token.allows("admin:read"));
        assert!(!token.allows("graph:write"));
    }

    #[test]
    fn rejects_bare_token_wildcard_default() {
        assert_eq!(ApiToken::parse("secret"), None);
        assert_eq!(ApiToken::parse("secret@tenant-a=*"), None);
        assert_eq!(ApiToken::parse("secret=graph:read"), None);
    }

    #[test]
    fn rejects_token_bound_to_different_tenant() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        let tokens = vec![ApiToken {
            token: "secret".to_string(),
            tenant_id: "tenant-a".to_string(),
            scopes: vec!["graph:write".to_string()],
        }];

        let result = require_scope_for_tenant(&headers, &tokens, "graph:write", true, "tenant-b");

        assert_eq!(result.unwrap_err(), StatusCode::FORBIDDEN);
    }
}
