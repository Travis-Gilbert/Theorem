//! Phases 2-4 + 8: the normalized field-fact, the per-field index policy, the
//! typed normalizer set, the field configuration (types, index policy, aliases,
//! composites, virtuals), and per-field masking.
//!
//! DATAWAVE references (`@ integration`):
//! - `NormalizedContentInterface` / `NormalizedFieldAndValue`: a field carries
//!   its name, original value, normalized value, grouping, and markings.
//! - `core/utils/type-utils/.../data/normalizer/`: a normalizer set gives each
//!   field a typed index-ready form. The numeric, date, and IP forms here match
//!   DATAWAVE's asserted test outputs (see the unit tests).
//! - `FieldConfigHelper` / `XMLFieldConfigHelper`: per-field index policy
//!   (indexed, reverse-indexed, tokenized, index-only).
//! - `CompositeIngest` (default separator U+10FFFF) / `VirtualIngest` (default
//!   separator space): composite/virtual derived fields.
//! - `FieldNameAliaserNormalizer`: external field names alias to internal ones.
//! - `MarkingsHelper` / `MaskedFieldHelper`: per-field visibility and a masked
//!   alternate value.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Per-field index policy. The four DATAWAVE flags decide how a field-fact
/// participates in retrieval; materialization writes them so the existing graph
/// index (and any future tiered global/field index) can read them.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct IndexPolicy {
    pub indexed: bool,
    pub reverse_indexed: bool,
    pub tokenized: bool,
    pub index_only: bool,
}

impl IndexPolicy {
    pub const NONE: IndexPolicy = IndexPolicy {
        indexed: false,
        reverse_indexed: false,
        tokenized: false,
        index_only: false,
    };

    /// Forward-indexed for value lookup, the common case.
    pub const INDEXED: IndexPolicy = IndexPolicy {
        indexed: true,
        reverse_indexed: false,
        tokenized: false,
        index_only: false,
    };

    pub fn indexed() -> Self {
        Self::INDEXED
    }

    pub fn with_reverse(mut self) -> Self {
        self.reverse_indexed = true;
        self
    }

    pub fn with_tokenized(mut self) -> Self {
        self.tokenized = true;
        self
    }

    pub fn with_index_only(mut self) -> Self {
        self.index_only = true;
        self
    }
}

impl Default for IndexPolicy {
    fn default() -> Self {
        Self::NONE
    }
}

/// The normalizer keyed by field type. Produces the index-ready normalized form.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Trim only; preserve case (a raw token).
    Text,
    /// Lowercase + strip diacritics, the DATAWAVE LcNoDiacriticsType /
    /// StringType default. No trimming, no whitespace collapse (DATAWAVE leaves
    /// those to the source parser).
    #[default]
    LcText,
    /// DATAWAVE NumberType / NumericalEncoder: a sign + exponent-letter + mantissa
    /// form whose ASCII sort order equals numeric order (`111` -> `+cE1.11`).
    Number,
    /// DATAWAVE DateType: parse a date/time, emit `yyyy-MM-ddTHH:mm:ss.SSSZ` (UTC).
    Date,
    /// DATAWAVE IpAddressNormalizer: zero-padded v4 octets (sortable), or v6.
    Ip,
    /// Parse `lat,lon`; emit canonical fixed-precision `lat,lon`.
    Geo,
    /// No normalization; pass the raw value through unchanged.
    NoOp,
}

impl FieldType {
    pub fn normalize(self, raw: &str) -> Result<String, NormalizeError> {
        match self {
            FieldType::NoOp => Ok(raw.to_string()),
            FieldType::Text => Ok(raw.trim().to_string()),
            FieldType::LcText => Ok(normalize_lc_text(raw)),
            FieldType::Number => normalize_number(raw),
            FieldType::Date => normalize_date(raw),
            FieldType::Ip => normalize_ip(raw),
            FieldType::Geo => normalize_geo(raw),
        }
    }
}

/// A value that could not be normalized to its declared field type. Carried,
/// not thrown: the driver drops the one field and the record still ingests.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NormalizeError {
    pub field_type: String,
    pub raw_value: String,
}

impl NormalizeError {
    pub fn new(field_type: &str, raw_value: &str) -> Self {
        Self {
            field_type: field_type.to_string(),
            raw_value: raw_value.to_string(),
        }
    }
}

impl fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot normalize {:?} as {}", self.raw_value, self.field_type)
    }
}

impl std::error::Error for NormalizeError {}

/// Lowercase + strip diacritics, matching DATAWAVE LcNoDiacriticsNormalizer.
/// No trim, no whitespace collapse: DATAWAVE's source parsers handle surrounding
/// whitespace, not the normalizer.
///
/// ponytail: ASCII-fold of the common Latin-1 accents instead of full Unicode
/// NFD decomposition. Covers the Cafe/naive class of inputs without a
/// `unicode-normalization` dep; swap in NFD if a corpus carries wider scripts.
pub fn normalize_lc_text(raw: &str) -> String {
    raw.chars().map(fold_diacritic).collect::<String>().to_lowercase()
}

/// Fold the common Latin-1 accented letters to their ASCII base.
fn fold_diacritic(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ñ' | 'Ñ' => 'n',
        'ç' | 'Ç' => 'c',
        other => other,
    }
}

// ---- Number normalization: DATAWAVE NumericalEncoder ----

fn normalize_number(raw: &str) -> Result<String, NormalizeError> {
    let s = raw.trim();
    // Idempotent: an already-encoded value passes through (DATAWAVE
    // isPossiblyEncoded).
    if is_possibly_encoded(s) {
        return Ok(s.to_string());
    }
    let n: f64 = s.parse().map_err(|_| NormalizeError::new("number", s))?;
    if !n.is_finite() {
        return Err(NormalizeError::new("number", s));
    }
    numerical_encode(n).ok_or_else(|| NormalizeError::new("number", s))
}

/// DATAWAVE `isPossiblyEncoded` regex `(!|+)[a-zA-Z][Ee][0-9].?[0-9]*`.
fn is_possibly_encoded(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 4
        && (b[0] == b'!' || b[0] == b'+')
        && b[1].is_ascii_alphabetic()
        && (b[2] == b'E' || b[2] == b'e')
        && b[3].is_ascii_digit()
}

/// Encode a finite f64 in DATAWAVE's lexicographically-sortable scientific form:
/// a sign char (`+`/`!`) + an exponent letter + `E` + the mantissa. Returns
/// `None` when the decimal exponent falls outside the encodable [-26, 25] range.
///
/// ponytail: f64 plus shortest-round-trip mantissa formatting reproduces every
/// asserted DATAWAVE NumericalEncoder output (see tests). A very long negative
/// mantissa could carry f64 subtraction noise that BigDecimal would not; the
/// `format_mantissa` rounding absorbs the common cases.
fn numerical_encode(v: f64) -> Option<String> {
    if v == 0.0 {
        return Some("+AE0".to_string());
    }
    let negative = v < 0.0;
    let sci = format!("{:e}", v.abs()); // "1.11e2", "1e0", "2.147483647e9"
    let (mantissa_str, exp_str) = sci.split_once('e')?;
    let exp: i32 = exp_str.parse().ok()?;

    let (prefix, letter) = if negative {
        if exp >= 0 {
            // magnitude >= 1: !A..!Z, exp 25 -> A, 0 -> Z.
            if exp > 25 {
                return None;
            }
            ('!', (b'A' + (25 - exp) as u8) as char)
        } else {
            // magnitude < 1: !a..!z, exp -1 -> a, -26 -> z.
            let idx = -exp - 1;
            if idx > 25 {
                return None;
            }
            ('!', (b'a' + idx as u8) as char)
        }
    } else if exp >= 0 {
        // positive magnitude >= 1: +a..+z, exp 0 -> a, 25 -> z.
        if exp > 25 {
            return None;
        }
        ('+', (b'a' + exp as u8) as char)
    } else {
        // positive magnitude < 1: +A..+Z, exp -1 -> Z, -26 -> A.
        let idx = exp + 26;
        if idx < 0 {
            return None;
        }
        ('+', (b'A' + idx as u8) as char)
    };

    let mantissa_out = if negative {
        // Negative mantissa is 10 + signed_coefficient (the coefficient is
        // negative), i.e. 10 - |coefficient|, so order reverses with magnitude.
        let coeff: f64 = mantissa_str.parse().ok()?;
        format_mantissa(10.0 - coeff)
    } else {
        mantissa_str.to_string()
    };
    Some(format!("{prefix}{letter}E{mantissa_out}"))
}

/// Format a mantissa with enough precision to absorb f64 subtraction noise, then
/// trim trailing zeros (DATAWAVE drops trailing zeros via its `#` DecimalFormat).
fn format_mantissa(m: f64) -> String {
    let s = format!("{m:.12}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_ip(raw: &str) -> Result<String, NormalizeError> {
    // DATAWAVE strips ALL spaces before parsing.
    let s: String = raw.chars().filter(|c| *c != ' ').collect();
    match s.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            let o = v4.octets();
            Ok(format!("{:03}.{:03}.{:03}.{:03}", o[0], o[1], o[2], o[3]))
        }
        Ok(std::net::IpAddr::V6(v6)) => Ok(v6.to_string()),
        // ponytail: DATAWAVE also normalizes wildcard octets (`1.2.3.*` ->
        // `001.002.003.*`) for query expansion; ingest field-facts carry real
        // addresses, so std parsing covers the write path.
        Err(_) => Err(NormalizeError::new("ip", &s)),
    }
}

fn normalize_geo(raw: &str) -> Result<String, NormalizeError> {
    let s = raw.trim();
    let (lat_s, lon_s) = s.split_once(',').ok_or_else(|| NormalizeError::new("geo", s))?;
    let lat: f64 = lat_s.trim().parse().map_err(|_| NormalizeError::new("geo", s))?;
    let lon: f64 = lon_s.trim().parse().map_err(|_| NormalizeError::new("geo", s))?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(NormalizeError::new("geo", s));
    }
    // ponytail: canonical fixed-precision lat,lon. A geohash / S2 cell id (the
    // DATAWAVE GeoNormalizer index form) lands by designating this field through
    // the core spatial_s2 index on the concrete store.
    Ok(format!("{lat:.6},{lon:.6}"))
}

// ---- Date normalization (no chrono; proleptic-Gregorian epoch math) ----

fn normalize_date(raw: &str) -> Result<String, NormalizeError> {
    let s = raw.trim();
    parse_date_to_ms(s)
        .map(epoch_ms_to_iso)
        .ok_or_else(|| NormalizeError::new("date", s))
}

/// Parse the DATAWAVE-relevant date forms to epoch milliseconds (UTC):
/// `yyyy-MM-dd['T'|' ']HH:mm[:ss[.SSS]]`, the pipe form `...'T'HH'|'mm`, the
/// compact `yyyyMMddHHmmss`, and an all-digit epoch-millis fallback.
/// ponytail: the textual `EEE MMM dd ... yyyy` form is a named follow-up; machine
/// data uses the numeric/ISO forms above.
fn parse_date_to_ms(s: &str) -> Option<i64> {
    let s = strip_trailing_zone(s.trim());
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        if s.len() == 14 {
            return compact14_to_ms(s); // yyyyMMddHHmmss
        }
        // DATAWAVE's fallback is new Date(long), i.e. epoch milliseconds.
        return s.parse::<i64>().ok();
    }
    let (date_part, time_part) = match s.split_once(['T', 't', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let (y, mo, d) = parse_ymd(date_part)?;
    let (hh, mi, ss, ms) = match time_part {
        Some(t) if !t.is_empty() => parse_hms(t)?,
        _ => (0, 0, 0, 0),
    };
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + hh * 3600 + mi * 60 + ss;
    Some(secs * 1000 + ms)
}

/// Strip a trailing `Z`/`z` and an alphabetic zone token (e.g. `GMT`).
fn strip_trailing_zone(s: &str) -> &str {
    let t = s.trim_end_matches(['Z', 'z']).trim_end();
    let bytes = t.as_bytes();
    let mut end = t.len();
    while end > 0 && bytes[end - 1].is_ascii_alphabetic() {
        end -= 1;
    }
    t[..end].trim_end()
}

fn compact14_to_ms(s: &str) -> Option<i64> {
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(4..6)?.parse().ok()?;
    let d: u32 = s.get(6..8)?.parse().ok()?;
    let hh: i64 = s.get(8..10)?.parse().ok()?;
    let mi: i64 = s.get(10..12)?.parse().ok()?;
    let ss: i64 = s.get(12..14)?.parse().ok()?;
    if !(1..=12).contains(&mo) || d == 0 || d > days_in_month(y, mo) || hh > 23 || mi > 59 || ss > 59 {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    Some((days * 86_400 + hh * 3600 + mi * 60 + ss) * 1000)
}

fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
    let mut p = s.split('-');
    let y: i64 = p.next()?.parse().ok()?;
    let mo: u32 = p.next()?.parse().ok()?;
    let d: u32 = p.next()?.parse().ok()?;
    if p.next().is_some() || !(1..=12).contains(&mo) || d == 0 || d > days_in_month(y, mo) {
        return None;
    }
    Some((y, mo, d))
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Calendar days in a month, so impossible dates (`2024-02-31`, `2023-02-29`)
/// are rejected rather than silently rolled forward by `days_from_civil`.
fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn parse_hms(t: &str) -> Option<(i64, i64, i64, i64)> {
    let (time, frac) = match t.split_once('.') {
        Some((tm, fr)) => (tm, Some(fr)),
        None => (t, None),
    };
    let mut parts = time.split([':', '|']);
    let hh: i64 = parts.next()?.parse().ok()?;
    let mi: i64 = parts.next().unwrap_or("0").parse().ok()?;
    let ss: i64 = parts.next().unwrap_or("0").parse().ok()?;
    if hh > 23 || mi > 59 || ss > 59 {
        return None;
    }
    let ms = match frac {
        // Truncate sub-second precision to milliseconds (DATAWAVE keeps `.SSS`).
        Some(fr) => {
            let digits: String = fr.chars().take_while(char::is_ascii_digit).take(3).collect();
            if digits.is_empty() {
                0
            } else {
                format!("{digits:0<3}").parse().ok()?
            }
        }
        None => 0,
    };
    Some((hh, mi, ss, ms))
}

fn epoch_ms_to_iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

/// Days since 1970-01-01 for a proleptic-Gregorian date (Howard Hinnant's
/// public-domain algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// How a field-fact came to exist: extracted from the record, or derived as a
/// composite (compound key) or virtual (single-source transform) field.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOrigin {
    #[default]
    Extracted,
    Composite,
    Virtual,
}

impl FieldOrigin {
    /// The authority layer the materialized fact carries: extracted fields are
    /// observed; composite/virtual fields are derived.
    pub fn authority(self) -> &'static str {
        match self {
            FieldOrigin::Extracted => "observed_fact",
            FieldOrigin::Composite | FieldOrigin::Virtual => "derived_fact",
        }
    }
}

/// The normalized field-fact: the unit DATAWAVE calls NormalizedFieldAndValue.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NormalizedField {
    /// Internal field name (after aliasing).
    pub field: String,
    pub raw_value: String,
    pub normalized: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Restricted alternate value for masked fields (DATAWAVE MaskedFieldHelper).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked: Option<String>,
    pub policy: IndexPolicy,
    pub field_type: FieldType,
    pub origin: FieldOrigin,
}

/// A composite field: concatenate the normalized values of `sources` into one
/// compound key under `name`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompositeDef {
    pub name: String,
    pub sources: Vec<String>,
    pub separator: String,
    #[serde(default)]
    pub policy: IndexPolicy,
}

impl CompositeDef {
    /// DATAWAVE composites join with `Character.MAX_CODE_POINT` (U+10FFFF) so the
    /// compound key cannot collide with a `\0`-using index separator.
    pub fn new(name: impl Into<String>, sources: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            name: name.into(),
            sources: sources.into_iter().map(Into::into).collect(),
            separator: "\u{10FFFF}".to_string(),
            policy: IndexPolicy::INDEXED,
        }
    }

    pub fn with_separator(mut self, sep: impl Into<String>) -> Self {
        self.separator = sep.into();
        self
    }
}

/// A virtual field: derive a new field `name` from a single `source` field via a
/// transform. (DATAWAVE virtual fields join multiple members with a space; the
/// single-source transform here is the common projection case.)
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VirtualDef {
    pub name: String,
    pub source: String,
    pub transform: VirtualTransform,
    #[serde(default)]
    pub policy: IndexPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum VirtualTransform {
    /// Copy the source field's normalized value verbatim.
    Copy,
    /// Lowercase the source's normalized value.
    Lowercase,
    /// Split the source on `delimiter` and take the component at `index`.
    Split { delimiter: char, index: usize },
}

impl VirtualTransform {
    pub fn apply(&self, value: &str) -> Option<String> {
        match self {
            VirtualTransform::Copy => Some(value.to_string()),
            VirtualTransform::Lowercase => Some(value.to_lowercase()),
            VirtualTransform::Split { delimiter, index } => {
                value.split(*delimiter).nth(*index).map(str::to_string)
            }
        }
    }
}

/// How a field is masked when its raw value is restricted.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum MaskRule {
    /// Replace the whole value with a fixed token.
    Redact { token: String },
    /// Keep only the last `keep` characters; prefix the rest with `*`.
    Last { keep: usize },
}

impl MaskRule {
    pub fn apply(&self, raw: &str) -> String {
        match self {
            MaskRule::Redact { token } => token.clone(),
            MaskRule::Last { keep } => {
                let chars: Vec<char> = raw.chars().collect();
                if chars.len() <= *keep {
                    return raw.to_string();
                }
                let masked_len = chars.len() - keep;
                let tail: String = chars[masked_len..].iter().collect();
                format!("{}{}", "*".repeat(masked_len), tail)
            }
        }
    }
}

/// The per-data-type field configuration: types, index policy, aliases,
/// composites, virtuals, and masking. The query side expands against the same
/// aliases written here.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct FieldConfig {
    types: BTreeMap<String, FieldType>,
    policies: BTreeMap<String, IndexPolicy>,
    aliases: BTreeMap<String, String>,
    masks: BTreeMap<String, MaskRule>,
    composites: Vec<CompositeDef>,
    virtuals: Vec<VirtualDef>,
    pub default_type: FieldType,
    pub default_policy: IndexPolicy,
}

impl FieldConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a field's type and index policy.
    pub fn with_field(mut self, name: impl Into<String>, field_type: FieldType, policy: IndexPolicy) -> Self {
        let name = name.into();
        self.types.insert(name.clone(), field_type);
        self.policies.insert(name, policy);
        self
    }

    /// Set the default index policy applied to fields with no explicit policy
    /// (the DATAWAVE "disallowlist" shape: index everything except the named).
    pub fn with_default_policy(mut self, policy: IndexPolicy) -> Self {
        self.default_policy = policy;
        self
    }

    pub fn with_default_type(mut self, field_type: FieldType) -> Self {
        self.default_type = field_type;
        self
    }

    /// Map an external field name to an internal one (DATAWAVE field aliasing).
    pub fn with_alias(mut self, external: impl Into<String>, internal: impl Into<String>) -> Self {
        self.aliases.insert(external.into(), internal.into());
        self
    }

    pub fn with_mask(mut self, field: impl Into<String>, rule: MaskRule) -> Self {
        self.masks.insert(field.into(), rule);
        self
    }

    pub fn with_composite(mut self, def: CompositeDef) -> Self {
        self.composites.push(def);
        self
    }

    pub fn with_virtual(mut self, def: VirtualDef) -> Self {
        self.virtuals.push(def);
        self
    }

    /// Resolve an external field name to its internal name (identity if no alias).
    pub fn resolve_alias<'a>(&'a self, external: &'a str) -> &'a str {
        self.aliases.get(external).map(String::as_str).unwrap_or(external)
    }

    pub fn field_type(&self, internal: &str) -> FieldType {
        self.types.get(internal).copied().unwrap_or(self.default_type)
    }

    pub fn policy(&self, internal: &str) -> IndexPolicy {
        self.policies.get(internal).copied().unwrap_or(self.default_policy)
    }

    pub fn mask_rule(&self, internal: &str) -> Option<&MaskRule> {
        self.masks.get(internal)
    }

    pub fn composites(&self) -> &[CompositeDef] {
        &self.composites
    }

    pub fn virtuals(&self) -> &[VirtualDef] {
        &self.virtuals
    }

    /// Every internal field name the config knows about. Feeds the dictionary.
    pub fn known_fields(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = self.types.keys().cloned().collect();
        names.extend(self.aliases.values().cloned());
        names.extend(self.composites.iter().map(|c| c.name.clone()));
        names.extend(self.virtuals.iter().map(|v| v.name.clone()));
        names.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lc_text_lowercases_and_folds_without_trimming() {
        // DATAWAVE LcNoDiacritics: lowercase + strip diacritics, single spaces kept.
        assert_eq!(normalize_lc_text("Café RÉSUMÉ"), "cafe resume");
        assert_eq!(normalize_lc_text("header_one"), "header_one");
        // No trim/collapse: surrounding and doubled whitespace is preserved.
        assert_eq!(normalize_lc_text(" Hello  World "), " hello  world ");
    }

    #[test]
    fn number_matches_datawave_numerical_encoder() {
        // Every pair below is asserted by DATAWAVE's NumberNormalizerTest /
        // NormalizedContentInterfaceTest.
        for (raw, want) in [
            ("1", "+aE1"),
            ("1.00000000", "+aE1"),
            ("0", "+AE0"),
            ("0.0", "+AE0"),
            ("0.00001", "+VE1"),
            ("111", "+cE1.11"),
            ("10000", "+eE1"),
            ("2147483647", "+jE2.147483647"),
            ("-1.0", "!ZE9"),
            ("-500", "!XE5"),
            ("-0.0001", "!dE9"),
            ("-0.0009", "!dE1"),
        ] {
            assert_eq!(FieldType::Number.normalize(raw).unwrap(), want, "encoding {raw}");
        }
        // Encoded order equals numeric order.
        assert!(FieldType::Number.normalize("-1.0").unwrap() < FieldType::Number.normalize("0").unwrap());
        assert!(FieldType::Number.normalize("0").unwrap() < FieldType::Number.normalize("0.00001").unwrap());
        assert!(FieldType::Number.normalize("abc").is_err());
    }

    #[test]
    fn ip_v4_is_zero_padded_and_space_stripped() {
        assert_eq!(FieldType::Ip.normalize("1.2.3.4").unwrap(), "001.002.003.004");
        assert_eq!(FieldType::Ip.normalize(" 1.2 .3.4 ").unwrap(), "001.002.003.004");
        assert!(FieldType::Ip.normalize("not-an-ip").is_err());
    }

    #[test]
    fn date_matches_datawave_outputs() {
        for (raw, want) in [
            ("2024-02-29 12:01:47", "2024-02-29T12:01:47.000Z"), // CSV fixture, leap day
            ("2014-10-20T17:20:20.001Z", "2014-10-20T17:20:20.001Z"),
            ("20141020172020", "2014-10-20T17:20:20.000Z"), // compact
            ("2014-10-20", "2014-10-20T00:00:00.000Z"),
            ("2014-10-20T17|20", "2014-10-20T17:20:00.000Z"), // pipe form
            ("2014-10-20 17:20:20GMT", "2014-10-20T17:20:20.000Z"),
            ("2014-10-20T17:20:20.345007Z", "2014-10-20T17:20:20.345Z"), // micros truncated
        ] {
            assert_eq!(FieldType::Date.normalize(raw).unwrap(), want, "date {raw}");
        }
        assert!(FieldType::Date.normalize("not-a-date").is_err());
    }

    #[test]
    fn date_rejects_impossible_calendar_dates() {
        // Silently rolling these into later real dates would index a wrong fact.
        assert!(FieldType::Date.normalize("2024-02-31").is_err());
        assert!(FieldType::Date.normalize("2023-04-31").is_err());
        assert!(FieldType::Date.normalize("2023-02-29").is_err()); // 2023 is not a leap year
        assert!(FieldType::Date.normalize("20230229000000").is_err()); // compact form too
        // The real leap day stays valid.
        assert_eq!(FieldType::Date.normalize("2024-02-29").unwrap(), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn geo_validates_ranges() {
        assert_eq!(FieldType::Geo.normalize("40.0,-105.0").unwrap(), "40.000000,-105.000000");
        assert!(FieldType::Geo.normalize("100.0,0.0").is_err());
    }

    #[test]
    fn mask_last_keeps_tail() {
        let rule = MaskRule::Last { keep: 4 };
        assert_eq!(rule.apply("4111111111111234"), "************1234");
    }

    #[test]
    fn config_resolves_alias_type_and_policy() {
        let cfg = FieldConfig::new()
            .with_alias("SRC_IP", "source_ip")
            .with_field("source_ip", FieldType::Ip, IndexPolicy::INDEXED);
        assert_eq!(cfg.resolve_alias("SRC_IP"), "source_ip");
        assert_eq!(cfg.field_type("source_ip"), FieldType::Ip);
        assert!(cfg.policy("source_ip").indexed);
    }
}
