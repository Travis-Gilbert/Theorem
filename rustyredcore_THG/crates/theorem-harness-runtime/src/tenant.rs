pub const DEFAULT_TENANT_SLUG: &str = "default";

pub fn normalize_tenant_slug(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        DEFAULT_TENANT_SLUG.to_string()
    } else {
        value.to_string()
    }
}

pub fn tenant_slug_aliases(value: &str) -> Vec<String> {
    let canonical = normalize_tenant_slug(value);
    let legacy_lowercase = canonical.to_lowercase();
    if legacy_lowercase == canonical {
        vec![canonical]
    } else {
        vec![canonical, legacy_lowercase]
    }
}

pub fn normalize_actor_id(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'))
        .trim()
        .to_string()
}
