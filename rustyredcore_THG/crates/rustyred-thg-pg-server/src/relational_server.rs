//! Spec-faithful planner-backed Postgres-wire surface (SPEC-RUSTYRED-PG-WIRE).
//!
//! This module is the native-views surface the spec describes: it parses SQL
//! with `sqlparser`, lowers it to the relational planner `QueryIr`, and executes
//! through the access-method seam (`execute_query` over a `RelationalStore`). It
//! is a sibling to the `serve()` entry point in `lib.rs` (which routes graph
//! reads through the `ThgExecutor` command surface); the two coexist while the
//! pg-server crate is co-edited live with Codex. The spec's SQL-to-planner
//! requirement lives here.
//!
//! Boundary (corrected in SPEC-RUSTYRED-PG-WIRE vs the relational-core spec):
//! this wire surface serves RustyRed's native relational VIEWS only (memory,
//! epistemic, graph relations). Auth/billing/accounts are the real-Postgres
//! escape hatch: the application connects to Neon directly for those, so the
//! native memory hot path (eviction/rehydration) never touches the wire.
//!
//! What this adds over the lib.rs server, against the acceptance criteria:
//! - SQL lowers to the planner `QueryIr` and runs through the access-method seam
//!   (AC3: joins of two native views).
//! - Modality predicates surfaced as functions: `time_range(col, lo, hi)`,
//!   `text_match(col, 'q')`, `knn(col, '[..]', k)`, `geo_within(col, ..)`
//!   (AC4: `time_range` routes to the `time_series` access method, provable via
//!   `EXPLAIN`).
//! - Row types inferred from `ScalarValue` so clients see real pg OIDs: text,
//!   int8, float8, bool (AC5).
//! - The extended protocol emits `ParameterDescription` before `RowDescription`
//!   so `tokio-postgres`/`psql` prepared statements work (AC2).
//! - `ORDER BY`, `LIMIT`/`OFFSET`, and `GROUP BY` with `count/sum/min/max/avg`
//!   handled in the wire frontend (the planner is conjunctive scan+join only).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use postgres_types::Type;
use rustyred_thg_core::{
    execute_query, JoinPredicate, Predicate, QueryIr, QueryRelation, RelationalStore, ScalarBound,
    ScalarValue,
};
use sqlparser::ast::{
    BinaryOperator, DuplicateTreatment, Expr, FunctionArg, FunctionArgExpr, FunctionArguments,
    GroupByExpr, JoinConstraint, JoinOperator, LimitClause, ObjectName, ObjectNamePart, OrderByKind,
    Query, Select, SelectItem, SetExpr, Statement, TableFactor, UnaryOperator, Value,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::{PgColumn, PgQueryResult};

/// The native-views relational store the wire surface serves, shared across
/// connections. Reads are short critical sections (lock, plan, collect, unlock).
pub type SharedRelationalStore = Arc<Mutex<RelationalStore>>;

const PROTOCOL_VERSION_3: i32 = 196608;
const SSL_REQUEST: i32 = 80877103;
const GSS_REQUEST: i32 = 80877104;
/// Upper bound on a single wire message body so a malicious/garbage length
/// cannot drive an unbounded allocation (a negative i32 length would otherwise
/// sign-extend to a huge usize).
const MAX_MESSAGE_LEN: i32 = 64 * 1024 * 1024;

// ----------------------------------------------------------------------------
// Errors carry a Postgres SQLSTATE so clients get a correct ErrorResponse.
// ----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PgError {
    pub sqlstate: String,
    pub message: String,
}

impl PgError {
    fn new(sqlstate: &str, message: impl Into<String>) -> Self {
        Self {
            sqlstate: sqlstate.to_string(),
            message: message.into(),
        }
    }
    fn syntax(message: impl Into<String>) -> Self {
        Self::new("42601", message)
    }
    fn undefined_table(message: impl Into<String>) -> Self {
        Self::new("42P01", message)
    }
    fn unsupported(message: impl Into<String>) -> Self {
        Self::new("0A000", message)
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new("XX000", message)
    }
}

type PgResult<T> = Result<T, PgError>;

// ----------------------------------------------------------------------------
// Server loop (blocking std::net, thread-per-connection, mirroring lib.rs).
// ----------------------------------------------------------------------------

/// Serve the native-views pg-wire surface over `listener`, backed by `store`.
pub fn serve_relational(listener: TcpListener, store: SharedRelationalStore) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        let store = Arc::clone(&store);
        thread::spawn(move || {
            let _ = handle_relational_stream(stream, store);
        });
    }
    Ok(())
}

/// Per-connection handler: startup handshake then the message loop covering both
/// the simple query protocol and the extended Parse/Bind/Describe/Execute/Sync.
pub fn handle_relational_stream(
    mut stream: TcpStream,
    store: SharedRelationalStore,
) -> std::io::Result<()> {
    if !read_startup(&mut stream)? {
        return Ok(()); // client disconnected during startup negotiation
    }
    stream.write_all(&startup_response())?;
    stream.flush()?;

    // Extended-protocol session state. prepared: stmt-name -> (sql, param OIDs).
    // portals: portal-name -> (param-substituted sql, result-format codes).
    let mut prepared: BTreeMap<String, PreparedStmt> = BTreeMap::new();
    let mut portals: BTreeMap<String, (String, Vec<i16>)> = BTreeMap::new();

    while let Some(message) = read_frontend_message(&mut stream)? {
        let mut out = Vec::new();
        match message {
            FrontendMessage::Query(sql) => {
                if sql.trim().is_empty() {
                    out.extend(empty_query_response());
                } else {
                    match run_sql(&sql, &store) {
                        Ok(result) => out.extend(encode_query_result(&result)),
                        Err(error) => out.extend(error_response(&error)),
                    }
                }
                out.extend(ready_for_query());
            }
            FrontendMessage::Parse { name, query, param_oids } => {
                prepared.insert(name, PreparedStmt { sql: query, param_oids });
                out.extend(parse_complete());
            }
            FrontendMessage::Bind {
                portal,
                statement,
                param_formats,
                param_values,
                result_formats,
            } => {
                // Resolve param OIDs from the prepared statement, render each bound
                // value to a SQL literal, and substitute $N -> literal so the
                // planner sees a concrete query.
                let (sql, param_oids) = prepared
                    .get(&statement)
                    .map(|stmt| (stmt.sql.clone(), stmt.param_oids.clone()))
                    .unwrap_or_else(|| (statement.clone(), Vec::new()));
                let literals: Vec<String> = param_values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let format = format_code_at(&param_formats, index);
                        let oid = param_oids.get(index).copied().unwrap_or(0);
                        render_param(value.as_deref(), format, oid)
                    })
                    .collect();
                let bound_sql = if literals.is_empty() {
                    sql
                } else {
                    substitute_placeholders(&sql, &literals)
                };
                portals.insert(portal, (bound_sql, result_formats));
                out.extend(bind_complete());
            }
            FrontendMessage::Describe { target, name } => {
                if target == b'S' {
                    // Statement describe: announce parameters, then the row shape.
                    // A parameterized SQL is not runnable yet, so probe it with
                    // NULLs only to recover the column names/shape.
                    let (sql, declared) = prepared
                        .get(&name)
                        .map(|stmt| (stmt.sql.clone(), stmt.param_oids.clone()))
                        .unwrap_or_default();
                    // Report one parameter per placeholder. Default undeclared (or
                    // 0/unspecified) params to TEXT rather than 0: a real driver
                    // resolves an unknown OID by introspecting pg_catalog.pg_type
                    // (a LEFT JOIN this minimal surface does not serve), so we
                    // report a built-in type the driver already knows. Declared
                    // non-zero types (e.g. from a typed ORM) are honored as-is.
                    let param_count = declared.len().max(max_placeholder(&sql));
                    let param_oids: Vec<u32> = (0..param_count)
                        .map(|i| match declared.get(i).copied() {
                            Some(oid) if oid != 0 => oid,
                            _ => Type::TEXT.oid(),
                        })
                        .collect();
                    out.extend(parameter_description(&param_oids));
                    if sql.trim().is_empty() {
                        out.extend(no_data());
                    } else {
                        let probe = probe_sql_for_describe(&sql);
                        let run_target = probe.as_deref().unwrap_or(sql.as_str());
                        match run_sql(run_target, &store) {
                            Ok(result) => out.extend(row_description(&result.columns)),
                            Err(error) => out.extend(error_response(&error)),
                        }
                    }
                } else {
                    // Portal describe: the SQL is already param-substituted.
                    let sql = portals.get(&name).map(|(sql, _)| sql.clone()).unwrap_or_default();
                    if sql.trim().is_empty() {
                        out.extend(no_data());
                    } else {
                        match run_sql(&sql, &store) {
                            Ok(result) => out.extend(row_description(&result.columns)),
                            Err(error) => out.extend(error_response(&error)),
                        }
                    }
                }
            }
            FrontendMessage::Execute { portal } => {
                let (sql, formats) = portals.get(&portal).cloned().unwrap_or_default();
                if sql.trim().is_empty() {
                    out.extend(empty_query_response());
                } else {
                    match run_sql(&sql, &store) {
                        // Extended Execute returns rows + tag only; the row shape
                        // was already sent by Describe. Values are encoded in the
                        // format the client requested in Bind (text or binary).
                        Ok(result) => {
                            for row in &result.rows {
                                out.extend(data_row_typed(row, &result.columns, &formats));
                            }
                            out.extend(command_complete(&result.command_tag));
                        }
                        Err(error) => out.extend(error_response(&error)),
                    }
                }
            }
            FrontendMessage::Sync => out.extend(ready_for_query()),
            FrontendMessage::Flush => {}
            FrontendMessage::Close => out.extend(close_complete()),
            FrontendMessage::Terminate => break,
            FrontendMessage::Password(_) => {
                // Trust auth: a stray password message is benign.
            }
        }
        if !out.is_empty() {
            stream.write_all(&out)?;
            stream.flush()?;
        }
    }
    Ok(())
}

fn run_sql(sql: &str, store: &SharedRelationalStore) -> PgResult<PgQueryResult> {
    let guard = store
        .lock()
        .map_err(|_| PgError::internal("relational store mutex poisoned"))?;
    execute_native_sql(sql, &guard)
}

// ----------------------------------------------------------------------------
// SQL -> planner QueryIr -> rows.
// ----------------------------------------------------------------------------

/// Parse `sql`, lower it to the planner IR, execute through the access-method
/// seam, and shape the result into typed Postgres rows. `EXPLAIN <select>`
/// returns the access-path plan (so `time_range` hitting the time-series index
/// is observable over the wire).
pub fn execute_native_sql(sql: &str, store: &RelationalStore) -> PgResult<PgQueryResult> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    let (explain, body) = match strip_keyword(trimmed, "explain") {
        Some(rest) => (true, rest.trim()),
        None => (false, trimmed),
    };

    let statements = Parser::parse_sql(&GenericDialect {}, body)
        .map_err(|error| PgError::syntax(error.to_string()))?;
    if statements.len() != 1 {
        return Err(PgError::syntax("expected exactly one statement"));
    }
    let query = match &statements[0] {
        Statement::Query(query) => query.as_ref(),
        other => {
            return Err(PgError::unsupported(format!(
                "only SELECT is supported by the native pg-wire surface, got: {}",
                statement_kind(other)
            )))
        }
    };

    let plan = lower_query(query)?;
    let result = execute_query(store, plan.ir.clone())
        .map_err(|error| PgError::internal(error.message))?;

    if explain {
        return Ok(explain_result(&result));
    }

    shape_result(plan, result)
}

/// A lowered query: the planner IR plus the wire-frontend post-processing
/// (projection, grouping, ordering, limit) the planner does not itself perform.
struct LoweredQuery {
    ir: QueryIr,
    projection: Vec<OutputColumn>,
    group_by: Vec<ResultKey>,
    order_by: Vec<OrderTerm>,
    limit: Option<usize>,
    offset: usize,
    select_star: bool,
}

#[derive(Clone)]
struct OutputColumn {
    display: String,
    source: ColumnSource,
}

#[derive(Clone)]
enum ColumnSource {
    /// A plain column read from the planner result, keyed `alias.column`.
    Key(ResultKey),
    /// An aggregate over an optional column (None for count(*)). `distinct` is
    /// set for `count(DISTINCT col)` and friends.
    Aggregate {
        kind: AggregateKind,
        arg: Option<ResultKey>,
        distinct: bool,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum AggregateKind {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

#[derive(Clone)]
struct OrderTerm {
    key: ResultKey,
    descending: bool,
}

/// A `alias.column` key into a planner result row.
type ResultKey = String;

/// One raw planner result row, keyed by `alias.column`.
type RawRow = BTreeMap<String, ScalarValue>;

fn lower_query(query: &Query) -> PgResult<LoweredQuery> {
    let SetExpr::Select(select) = query.body.as_ref() else {
        return Err(PgError::unsupported(
            "only plain SELECT bodies are supported (no UNION/VALUES)",
        ));
    };
    let select: &Select = select.as_ref();

    if select.from.is_empty() {
        return Err(PgError::unsupported(
            "SELECT without FROM is not a native view query",
        ));
    }

    // FROM: the root relation plus INNER joins.
    let from = &select.from[0];
    if select.from.len() > 1 {
        return Err(PgError::unsupported(
            "comma-joined FROM is not supported; use explicit JOIN ... ON",
        ));
    }
    let (root_relation, root_alias) = table_factor(&from.relation)?;
    let mut relations = vec![QueryRelation {
        alias: root_alias.clone(),
        relation: root_relation,
        predicates: Vec::new(),
    }];
    let mut joins = Vec::new();
    for join in &from.joins {
        let (relation, alias) = table_factor(&join.relation)?;
        let constraint = match &join.join_operator {
            JoinOperator::Inner(c) | JoinOperator::Join(c) => c,
            _ => return Err(PgError::unsupported("only INNER JOIN is supported")),
        };
        let JoinConstraint::On(on) = constraint else {
            return Err(PgError::unsupported("JOIN requires an ON <a.x = b.y> clause"));
        };
        let (left, right) = equi_join_columns(on, &root_alias, &alias)?;
        relations.push(QueryRelation {
            alias: alias.clone(),
            relation,
            predicates: Vec::new(),
        });
        joins.push(JoinPredicate {
            left_alias: left.0,
            left_column: left.1,
            right_alias: right.0,
            right_column: right.1,
        });
    }

    // WHERE: AND-decompose into per-relation predicates.
    if let Some(selection) = &select.selection {
        let mut predicates: Vec<(String, Predicate)> = Vec::new();
        decompose_where(selection, &root_alias, &mut predicates)?;
        for (alias, predicate) in predicates {
            let target = relations
                .iter_mut()
                .find(|relation| relation.alias == alias)
                .ok_or_else(|| {
                    PgError::syntax(format!("predicate references unknown relation alias '{alias}'"))
                })?;
            target.predicates.push(predicate);
        }
    }

    // Projection / aggregates.
    let mut projection = Vec::new();
    let mut select_star = false;
    for item in &select.projection {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => select_star = true,
            SelectItem::UnnamedExpr(expr) => {
                projection.push(output_column(expr, None, &root_alias)?)
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                projection.push(output_column(expr, Some(ident_name(alias)), &root_alias)?)
            }
            SelectItem::ExprWithAliases { .. } => {
                return Err(PgError::unsupported("multi-alias projection is not supported"))
            }
        }
    }

    let group_by = match &select.group_by {
        GroupByExpr::Expressions(exprs, _) => exprs
            .iter()
            .map(|expr| {
                column_ref(expr, &root_alias)
                    .map(|(a, c)| format!("{a}.{c}"))
                    .ok_or_else(|| PgError::syntax("GROUP BY must reference a column"))
            })
            .collect::<PgResult<Vec<_>>>()?,
        GroupByExpr::All(_) => {
            return Err(PgError::unsupported("GROUP BY ALL is not supported"))
        }
    };

    let order_by = lower_order_by(query, &root_alias)?;
    let (limit, offset) = lower_limit(query)?;

    Ok(LoweredQuery {
        ir: QueryIr {
            relations,
            joins,
            projection: Vec::new(), // fetch all columns; we project in the frontend
            limit: None,            // ORDER BY/GROUP BY must see all rows first
        },
        projection,
        group_by,
        order_by,
        limit,
        offset,
        select_star,
    })
}

fn lower_order_by(query: &Query, root_alias: &str) -> PgResult<Vec<OrderTerm>> {
    let Some(order_by) = &query.order_by else {
        return Ok(Vec::new());
    };
    let OrderByKind::Expressions(terms) = &order_by.kind else {
        return Err(PgError::unsupported("ORDER BY ALL is not supported"));
    };
    terms
        .iter()
        .map(|term| {
            let (alias, column) = column_ref(&term.expr, root_alias)
                .ok_or_else(|| PgError::syntax("ORDER BY must reference a column"))?;
            Ok(OrderTerm {
                key: format!("{alias}.{column}"),
                descending: term.options.asc == Some(false),
            })
        })
        .collect()
}

fn lower_limit(query: &Query) -> PgResult<(Option<usize>, usize)> {
    let Some(clause) = &query.limit_clause else {
        return Ok((None, 0));
    };
    match clause {
        LimitClause::LimitOffset { limit, offset, .. } => {
            let limit = match limit {
                Some(expr) => Some(usize_literal(expr)?),
                None => None,
            };
            let offset = match offset {
                Some(offset) => usize_literal(&offset.value)?,
                None => 0,
            };
            Ok((limit, offset))
        }
        LimitClause::OffsetCommaLimit { offset, limit } => {
            Ok((Some(usize_literal(limit)?), usize_literal(offset)?))
        }
    }
}

fn output_column(expr: &Expr, alias: Option<String>, root: &str) -> PgResult<OutputColumn> {
    if let Some((kind, arg, distinct)) = aggregate_call(expr, root)? {
        let display = alias.unwrap_or_else(|| aggregate_name(kind).to_string());
        return Ok(OutputColumn {
            display,
            source: ColumnSource::Aggregate { kind, arg, distinct },
        });
    }
    let (col_alias, column) = column_ref(expr, root)
        .ok_or_else(|| PgError::unsupported("projection must be a column or aggregate"))?;
    Ok(OutputColumn {
        display: alias.unwrap_or_else(|| column.clone()),
        source: ColumnSource::Key(format!("{col_alias}.{column}")),
    })
}

/// Recognize `count(*) | count([DISTINCT] col) | sum/min/max/avg([DISTINCT] col)`.
fn aggregate_call(
    expr: &Expr,
    root: &str,
) -> PgResult<Option<(AggregateKind, Option<ResultKey>, bool)>> {
    let Expr::Function(function) = expr else {
        return Ok(None);
    };
    let name = object_name_lower(&function.name);
    let kind = match name.as_str() {
        "count" => AggregateKind::Count,
        "sum" => AggregateKind::Sum,
        "min" => AggregateKind::Min,
        "max" => AggregateKind::Max,
        "avg" => AggregateKind::Avg,
        _ => return Ok(None),
    };
    // Read DISTINCT off the argument list so count(DISTINCT col) is not silently
    // lowered to count(col).
    let (distinct, args) = match &function.args {
        FunctionArguments::List(list) => (
            matches!(list.duplicate_treatment, Some(DuplicateTreatment::Distinct)),
            list.args.clone(),
        ),
        _ => (false, Vec::new()),
    };
    // count(*) has a wildcard arg.
    if matches!(kind, AggregateKind::Count)
        && args
            .iter()
            .any(|arg| matches!(arg, FunctionArg::Unnamed(FunctionArgExpr::Wildcard)))
    {
        return Ok(Some((kind, None, distinct)));
    }
    let column_expr = args.iter().find_map(unnamed_expr).ok_or_else(|| {
        PgError::syntax(format!("aggregate {name} expects a column argument"))
    })?;
    let (alias, column) = column_ref(column_expr, root)
        .ok_or_else(|| PgError::syntax(format!("aggregate {name} argument must be a column")))?;
    Ok(Some((kind, Some(format!("{alias}.{column}")), distinct)))
}

// ----------------------------------------------------------------------------
// WHERE -> predicates (scalar + modality functions).
// ----------------------------------------------------------------------------

fn decompose_where(
    expr: &Expr,
    root: &str,
    out: &mut Vec<(String, Predicate)>,
) -> PgResult<()> {
    match expr {
        Expr::Nested(inner) => decompose_where(inner, root, out),
        Expr::BinaryOp { left, op: BinaryOperator::And, right } => {
            decompose_where(left, root, out)?;
            decompose_where(right, root, out)
        }
        Expr::BinaryOp { op: BinaryOperator::Or, .. } => Err(PgError::unsupported(
            "OR is not supported; the planner conjuncts predicates",
        )),
        Expr::BinaryOp { left, op, right } => {
            let (alias, predicate) = comparison_predicate(left, op, right, root)?;
            out.push((alias, predicate));
            Ok(())
        }
        Expr::Between { expr, negated: false, low, high } => {
            let (alias, column) = column_ref(expr, root)
                .ok_or_else(|| PgError::syntax("BETWEEN must apply to a column"))?;
            let lo = scalar_literal(low)?;
            let hi = scalar_literal(high)?;
            out.push((
                alias,
                Predicate::Range {
                    column,
                    lo: ScalarBound::Included(lo),
                    hi: ScalarBound::Included(hi),
                },
            ));
            Ok(())
        }
        Expr::Like { expr, pattern, negated: false, .. } => {
            let (alias, column) = column_ref(expr, root)
                .ok_or_else(|| PgError::syntax("LIKE must apply to a column"))?;
            let prefix = like_prefix(pattern)?;
            out.push((alias, Predicate::Prefix { column, prefix }));
            Ok(())
        }
        Expr::Function(_) => {
            let (alias, predicate) = modality_predicate(expr, root)?;
            out.push((alias, predicate));
            Ok(())
        }
        other => Err(PgError::unsupported(format!(
            "unsupported WHERE expression: {other}"
        ))),
    }
}

fn comparison_predicate(
    left: &Expr,
    op: &BinaryOperator,
    right: &Expr,
    root: &str,
) -> PgResult<(String, Predicate)> {
    // Accept `col <op> literal` and `literal <op> col` (operator flipped).
    let (alias, column, value, flipped) = if let Some((alias, column)) = column_ref(left, root) {
        (alias, column, scalar_literal(right)?, false)
    } else if let Some((alias, column)) = column_ref(right, root) {
        (alias, column, scalar_literal(left)?, true)
    } else {
        return Err(PgError::syntax("comparison must involve a column and a literal"));
    };
    let op = if flipped { flip_operator(op) } else { op.clone() };
    let predicate = match op {
        BinaryOperator::Eq => Predicate::Equals { column, value },
        BinaryOperator::Gt => Predicate::Range {
            column,
            lo: ScalarBound::Excluded(value),
            hi: ScalarBound::Unbounded,
        },
        BinaryOperator::GtEq => Predicate::Range {
            column,
            lo: ScalarBound::Included(value),
            hi: ScalarBound::Unbounded,
        },
        BinaryOperator::Lt => Predicate::Range {
            column,
            lo: ScalarBound::Unbounded,
            hi: ScalarBound::Excluded(value),
        },
        BinaryOperator::LtEq => Predicate::Range {
            column,
            lo: ScalarBound::Unbounded,
            hi: ScalarBound::Included(value),
        },
        BinaryOperator::NotEq => {
            return Err(PgError::unsupported("<> is not supported by the ordered index"))
        }
        other => {
            return Err(PgError::unsupported(format!(
                "unsupported comparison operator {other:?}"
            )))
        }
    };
    Ok((alias, predicate))
}

fn flip_operator(op: &BinaryOperator) -> BinaryOperator {
    match op {
        BinaryOperator::Gt => BinaryOperator::Lt,
        BinaryOperator::Lt => BinaryOperator::Gt,
        BinaryOperator::GtEq => BinaryOperator::LtEq,
        BinaryOperator::LtEq => BinaryOperator::GtEq,
        other => other.clone(),
    }
}

/// Modality predicates surfaced as SQL functions in the WHERE clause.
///
/// Only `time_range` is honored: the relational planner registers an ordered +
/// time-series access method, so `time_range` hits a real index. `knn`,
/// `geo_within`, and `text_match` are REJECTED rather than executed, because the
/// planner currently stubs them (row_matches returns true unconditionally),
/// which would silently return unfiltered rows. Wiring the vector / spatial /
/// fulltext access methods into the relational registry is a core (planner)
/// change; until then this surface refuses them instead of lying.
fn modality_predicate(expr: &Expr, root: &str) -> PgResult<(String, Predicate)> {
    let Expr::Function(function) = expr else {
        return Err(PgError::syntax("expected a modality predicate function"));
    };
    let name = object_name_lower(&function.name);
    let fn_args = function_args(&function.args);
    let args: Vec<&Expr> = fn_args.iter().filter_map(unnamed_expr).collect();
    match name.as_str() {
        "time_range" => {
            require_arity(&name, &args, 3)?;
            let (alias, column) = args
                .first()
                .and_then(|expr| column_ref(expr, root))
                .ok_or_else(|| {
                    PgError::syntax("time_range() first argument must be a column")
                })?;
            Ok((
                alias,
                Predicate::TimeRange {
                    column,
                    lo_ms: int_literal(args[1])?,
                    hi_ms: int_literal(args[2])?,
                },
            ))
        }
        "knn" | "geo_within" | "text_match" => Err(PgError::unsupported(format!(
            "{name}() is recognized but not yet backed by a relational access method \
             (only time_range is indexed); the vector/spatial/fulltext methods are not wired \
             into the planner registry, so refusing rather than returning unfiltered rows"
        ))),
        other => Err(PgError::unsupported(format!(
            "unknown predicate function '{other}'"
        ))),
    }
}

fn require_arity(name: &str, args: &[&Expr], expected: usize) -> PgResult<()> {
    if args.len() != expected {
        return Err(PgError::syntax(format!(
            "{name}() expects {expected} arguments, got {}",
            args.len()
        )));
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Result shaping: projection, grouping, ordering, limit, type inference.
// ----------------------------------------------------------------------------

fn shape_result(
    plan: LoweredQuery,
    result: rustyred_thg_core::QueryResult,
) -> PgResult<PgQueryResult> {
    let raw_rows = result.rows;

    // Resolve the output column set.
    let output: Vec<OutputColumn> = if plan.select_star && plan.projection.is_empty() {
        star_columns(&raw_rows)
    } else {
        plan.projection.clone()
    };

    let has_aggregate = output
        .iter()
        .any(|column| matches!(column.source, ColumnSource::Aggregate { .. }));

    // ORDER BY ordering depends on whether the sort columns survive projection.
    // Non-aggregate queries may ORDER BY a column not in the SELECT list, so sort
    // the RAW planner rows (keyed `alias.column`) before projecting. Aggregate /
    // grouped queries ORDER BY output columns, so sort the shaped rows.
    // shaped rows are POSITIONAL: each inner vec aligns 1:1 with `output`, so two
    // output columns that share a display name (e.g. a join's two `id` columns)
    // never collide.
    let mut shaped: Vec<Vec<Option<ScalarValue>>> =
        if has_aggregate || !plan.group_by.is_empty() {
            let mut grouped = aggregate_rows(&raw_rows, &output, &plan.group_by)?;
            if !plan.order_by.is_empty() {
                sort_positional(&mut grouped, &positional_order_terms(&plan.order_by, &output));
            }
            grouped
        } else {
            let mut rows = raw_rows;
            if !plan.order_by.is_empty() {
                let order: Vec<(String, bool)> = plan
                    .order_by
                    .iter()
                    .map(|term| (term.key.clone(), term.descending))
                    .collect();
                sort_raw(&mut rows, &order);
            }
            rows.iter().map(|row| project_row(row, &output)).collect()
        };

    // OFFSET / LIMIT.
    if plan.offset > 0 {
        shaped = shaped.into_iter().skip(plan.offset).collect();
    }
    if let Some(limit) = plan.limit {
        shaped.truncate(limit);
    }

    // Build columns with inferred OIDs + text-encoded rows (positional).
    let columns = infer_columns(&output, &shaped);
    let rows: Vec<Vec<Option<String>>> = shaped
        .iter()
        .map(|row| row.iter().map(|value| value.as_ref().map(scalar_to_text)).collect())
        .collect();

    Ok(PgQueryResult {
        command_tag: format!("SELECT {}", rows.len()),
        columns,
        rows,
        trace: serde_json::to_value(result.trace).ok(),
    })
}

/// `SELECT *`: derive the column set from the union of keys in the result rows.
fn star_columns(rows: &[BTreeMap<String, ScalarValue>]) -> Vec<OutputColumn> {
    let mut keys: Vec<String> = rows
        .iter()
        .flat_map(|row| row.keys().cloned())
        .collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .map(|key| {
            let display = key.rsplit('.').next().unwrap_or(&key).to_string();
            OutputColumn {
                display,
                source: ColumnSource::Key(key),
            }
        })
        .collect()
}

/// Project one raw planner row into a positional output row aligned with `output`.
fn project_row(
    row: &BTreeMap<String, ScalarValue>,
    output: &[OutputColumn],
) -> Vec<Option<ScalarValue>> {
    output
        .iter()
        .map(|column| match &column.source {
            ColumnSource::Key(key) => row.get(key).cloned(),
            // aggregates never appear on the non-aggregate path
            ColumnSource::Aggregate { .. } => None,
        })
        .collect()
}

fn aggregate_rows(
    raw_rows: &[BTreeMap<String, ScalarValue>],
    output: &[OutputColumn],
    group_by: &[ResultKey],
) -> PgResult<Vec<Vec<Option<ScalarValue>>>> {
    // Partition rows into groups keyed by the GROUP BY column values.
    let mut groups: Vec<(Vec<String>, Vec<&RawRow>)> = Vec::new();
    for row in raw_rows {
        let key: Vec<String> = group_by
            .iter()
            .map(|gk| row.get(gk).map(scalar_to_text).unwrap_or_default())
            .collect();
        match groups.iter_mut().find(|(existing, _)| existing == &key) {
            Some((_, members)) => members.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    // With aggregates but no GROUP BY, fold everything into one group.
    if group_by.is_empty() && groups.is_empty() {
        groups.push((Vec::new(), Vec::new()));
    }

    let mut shaped = Vec::new();
    for (_, members) in groups {
        let row: Vec<Option<ScalarValue>> = output
            .iter()
            .map(|column| match &column.source {
                ColumnSource::Aggregate { kind, arg, distinct } => {
                    Some(aggregate_value(*kind, arg.as_deref(), *distinct, &members))
                }
                ColumnSource::Key(key) => members.first().and_then(|row| row.get(key).cloned()),
            })
            .collect();
        shaped.push(row);
    }
    Ok(shaped)
}

fn aggregate_value(
    kind: AggregateKind,
    arg: Option<&str>,
    distinct: bool,
    members: &[&BTreeMap<String, ScalarValue>],
) -> ScalarValue {
    let mut values: Vec<&ScalarValue> = match arg {
        Some(key) => members.iter().filter_map(|row| row.get(key)).collect(),
        None => Vec::new(),
    };
    if distinct {
        // DISTINCT dedupes by text rendering (ScalarValue has no total order).
        let mut seen = std::collections::BTreeSet::new();
        values.retain(|value| seen.insert(scalar_to_text(value)));
    }
    match kind {
        AggregateKind::Count => {
            // count(*) counts member rows; count([DISTINCT] col) counts the
            // (deduped) non-null values.
            let count = match arg {
                Some(_) => values.len(),
                None => members.len(),
            };
            ScalarValue::I64(count as i64)
        }
        AggregateKind::Sum => {
            let sum: f64 = values.iter().filter_map(|value| value.as_f64()).sum();
            numeric_scalar(sum)
        }
        AggregateKind::Avg => {
            let nums: Vec<f64> = values.iter().filter_map(|value| value.as_f64()).collect();
            if nums.is_empty() {
                ScalarValue::String(String::new())
            } else {
                ScalarValue::F64(nums.iter().sum::<f64>() / nums.len() as f64)
            }
        }
        AggregateKind::Min | AggregateKind::Max => {
            extreme(&values, matches!(kind, AggregateKind::Max))
        }
    }
}

fn extreme(values: &[&ScalarValue], want_max: bool) -> ScalarValue {
    let mut best: Option<ScalarValue> = None;
    for value in values {
        let replace = match &best {
            None => true,
            Some(current) => {
                let ordering = compare_scalar(current, value);
                if want_max {
                    ordering == std::cmp::Ordering::Less
                } else {
                    ordering == std::cmp::Ordering::Greater
                }
            }
        };
        if replace {
            best = Some((*value).clone());
        }
    }
    best.unwrap_or(ScalarValue::String(String::new()))
}

/// An integral sum stays int8; a fractional sum becomes float8.
fn numeric_scalar(value: f64) -> ScalarValue {
    if value.fract() == 0.0 && value.abs() < i64::MAX as f64 {
        ScalarValue::I64(value as i64)
    } else {
        ScalarValue::F64(value)
    }
}

/// Sort RAW planner rows (keyed `alias.column`) for the non-aggregate path.
fn sort_raw(rows: &mut [BTreeMap<String, ScalarValue>], order: &[(String, bool)]) {
    rows.sort_by(|a, b| {
        for (key, descending) in order {
            let ordering = match (a.get(key), b.get(key)) {
                (Some(left), Some(right)) => compare_scalar(left, right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };
            let ordering = if *descending { ordering.reverse() } else { ordering };
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }
        std::cmp::Ordering::Equal
    });
}

/// Sort POSITIONAL shaped rows by output-column index (aggregate / grouped path).
fn sort_positional(rows: &mut [Vec<Option<ScalarValue>>], order: &[(usize, bool)]) {
    rows.sort_by(|a, b| {
        for (index, descending) in order {
            let left = a.get(*index).and_then(|value| value.as_ref());
            let right = b.get(*index).and_then(|value| value.as_ref());
            let ordering = match (left, right) {
                (Some(left), Some(right)) => compare_scalar(left, right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };
            let ordering = if *descending { ordering.reverse() } else { ordering };
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }
        std::cmp::Ordering::Equal
    });
}

/// Map each ORDER BY result key to the matching output column INDEX.
fn positional_order_terms(order_by: &[OrderTerm], output: &[OutputColumn]) -> Vec<(usize, bool)> {
    order_by
        .iter()
        .filter_map(|term| output_index_for_key(&term.key, output).map(|idx| (idx, term.descending)))
        .collect()
}

fn output_index_for_key(key: &str, output: &[OutputColumn]) -> Option<usize> {
    let bare = key.rsplit('.').next().unwrap_or(key);
    output.iter().position(|column| match &column.source {
        ColumnSource::Key(k) => k == key || column.display == bare,
        ColumnSource::Aggregate { .. } => column.display == bare,
    })
}

fn compare_scalar(a: &ScalarValue, b: &ScalarValue) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(std::cmp::Ordering::Equal),
        _ => scalar_to_text(a).cmp(&scalar_to_text(b)),
    }
}

/// Infer one pg type per output column from the values actually present.
fn infer_columns(output: &[OutputColumn], rows: &[Vec<Option<ScalarValue>>]) -> Vec<PgColumn> {
    output
        .iter()
        .enumerate()
        .map(|(index, column)| {
            // count(*) is always int8 even over zero rows.
            if let ColumnSource::Aggregate { kind: AggregateKind::Count, .. } = column.source {
                return PgColumn::int8(&column.display);
            }
            let mut seen_int = false;
            let mut seen_float = false;
            let mut seen_bool = false;
            let mut seen_text = false;
            let mut seen_any = false;
            for row in rows {
                if let Some(value) = row.get(index).and_then(|value| value.as_ref()) {
                    seen_any = true;
                    match value {
                        ScalarValue::I64(_) => seen_int = true,
                        ScalarValue::F64(_) => seen_float = true,
                        ScalarValue::Bool(_) => seen_bool = true,
                        ScalarValue::String(_) => seen_text = true,
                    }
                }
            }
            if !seen_any || seen_text {
                PgColumn::text(&column.display)
            } else if seen_bool && !seen_int && !seen_float {
                PgColumn::bool(&column.display)
            } else if seen_float {
                PgColumn::float8(&column.display)
            } else if seen_int {
                PgColumn::int8(&column.display)
            } else {
                PgColumn::text(&column.display)
            }
        })
        .collect()
}

/// `EXPLAIN`: render the planner trace as a single-column result so a SQL client
/// can see which access method served each predicate (e.g. `time_series`).
fn explain_result(result: &rustyred_thg_core::QueryResult) -> PgQueryResult {
    let trace = &result.trace;
    let mut lines = Vec::new();
    for path in &trace.access_paths {
        lines.push(format!(
            "{} via {} (est_rows={:.1} returned={} visited={})",
            path.predicate, path.method, path.est_rows, path.returned_rows, path.visited_rows
        ));
    }
    lines.push(format!(
        "full_relation_scans={} bitmap_intersections={} roaring={} join={} rows={}",
        trace.full_relation_scans,
        trace.bitmap_intersections,
        trace.used_roaring_bitmaps,
        trace.join_algorithm.clone().unwrap_or_else(|| "none".to_string()),
        trace.joined_rows,
    ));
    let rows = lines
        .into_iter()
        .map(|line| vec![Some(line)])
        .collect::<Vec<_>>();
    let count = rows.len();
    PgQueryResult {
        columns: vec![PgColumn::text("QUERY PLAN")],
        rows,
        command_tag: format!("EXPLAIN {count}"),
        trace: serde_json::to_value(&result.trace).ok(),
    }
}

// ----------------------------------------------------------------------------
// AST helpers.
// ----------------------------------------------------------------------------

fn table_factor(factor: &TableFactor) -> PgResult<(String, String)> {
    let TableFactor::Table { name, alias, .. } = factor else {
        return Err(PgError::unsupported("only plain table relations are supported"));
    };
    let relation = object_name_lower(name);
    if relation.is_empty() {
        return Err(PgError::undefined_table("empty relation name"));
    }
    let alias = alias
        .as_ref()
        .map(|alias| ident_name(&alias.name))
        .unwrap_or_else(|| relation.clone());
    Ok((relation, alias))
}

fn equi_join_columns(
    on: &Expr,
    left_alias: &str,
    right_alias: &str,
) -> PgResult<((String, String), (String, String))> {
    let Expr::BinaryOp { left, op: BinaryOperator::Eq, right } = on else {
        return Err(PgError::unsupported("JOIN ON must be a single equality"));
    };
    let lhs = column_ref(left, left_alias)
        .ok_or_else(|| PgError::syntax("JOIN ON left side must be a column"))?;
    let rhs = column_ref(right, right_alias)
        .ok_or_else(|| PgError::syntax("JOIN ON right side must be a column"))?;
    // Ensure left tuple belongs to the left relation; otherwise swap.
    if lhs.0 == right_alias && rhs.0 == left_alias {
        Ok((rhs, lhs))
    } else {
        Ok((lhs, rhs))
    }
}

fn column_ref(expr: &Expr, root: &str) -> Option<(String, String)> {
    match expr {
        Expr::Identifier(id) => Some((root.to_string(), ident_name(id))),
        Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
            Some((ident_name(&parts[0]), ident_name(&parts[1])))
        }
        Expr::Nested(inner) => column_ref(inner, root),
        _ => None,
    }
}

fn scalar_literal(expr: &Expr) -> PgResult<ScalarValue> {
    literal(expr).ok_or_else(|| PgError::syntax(format!("expected a literal value, got: {expr}")))
}

fn literal(expr: &Expr) -> Option<ScalarValue> {
    match expr {
        Expr::Value(value) => value_to_scalar(&value.value),
        Expr::Nested(inner) => literal(inner),
        Expr::UnaryOp { op: UnaryOperator::Minus, expr } => match literal(expr)? {
            ScalarValue::I64(value) => Some(ScalarValue::I64(-value)),
            ScalarValue::F64(value) => Some(ScalarValue::F64(-value)),
            _ => None,
        },
        _ => None,
    }
}

fn value_to_scalar(value: &Value) -> Option<ScalarValue> {
    match value {
        Value::Number(number, _) => number
            .parse::<i64>()
            .map(ScalarValue::I64)
            .ok()
            .or_else(|| number.parse::<f64>().map(ScalarValue::F64).ok()),
        Value::SingleQuotedString(string) | Value::DoubleQuotedString(string) => {
            Some(ScalarValue::String(string.clone()))
        }
        Value::Boolean(boolean) => Some(ScalarValue::Bool(*boolean)),
        _ => None,
    }
}

fn int_literal(expr: &Expr) -> PgResult<i64> {
    scalar_literal(expr)?
        .as_i64()
        .ok_or_else(|| PgError::syntax("expected an integer literal"))
}

fn usize_literal(expr: &Expr) -> PgResult<usize> {
    let value = int_literal(expr)?;
    usize::try_from(value).map_err(|_| PgError::syntax("expected a non-negative integer"))
}

fn string_literal(expr: &Expr) -> PgResult<String> {
    match scalar_literal(expr)? {
        ScalarValue::String(string) => Ok(string),
        other => Ok(scalar_to_text(&other)),
    }
}

fn like_prefix(pattern: &Expr) -> PgResult<String> {
    let raw = string_literal(pattern)?;
    let Some(prefix) = raw.strip_suffix('%') else {
        return Err(PgError::unsupported(
            "only prefix LIKE patterns ('text%') are supported",
        ));
    };
    if prefix.contains('%') || prefix.contains('_') {
        return Err(PgError::unsupported(
            "only a single trailing-% LIKE pattern is supported",
        ));
    }
    Ok(prefix.to_string())
}

fn function_args(args: &FunctionArguments) -> Vec<FunctionArg> {
    match args {
        FunctionArguments::List(list) => list.args.clone(),
        _ => Vec::new(),
    }
}

fn unnamed_expr(arg: &FunctionArg) -> Option<&Expr> {
    match arg {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr),
        _ => None,
    }
}

fn ident_name(ident: &sqlparser::ast::Ident) -> String {
    // Postgres folds unquoted identifiers to lower case.
    if ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_ascii_lowercase()
    }
}

fn object_name_lower(name: &ObjectName) -> String {
    name.0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident_name(ident)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn aggregate_name(kind: AggregateKind) -> &'static str {
    match kind {
        AggregateKind::Count => "count",
        AggregateKind::Sum => "sum",
        AggregateKind::Min => "min",
        AggregateKind::Max => "max",
        AggregateKind::Avg => "avg",
    }
}

fn statement_kind(statement: &Statement) -> &'static str {
    match statement {
        Statement::Insert(_) => "INSERT",
        Statement::Update { .. } => "UPDATE",
        Statement::Delete(_) => "DELETE",
        _ => "non-SELECT statement",
    }
}

fn strip_keyword<'a>(value: &'a str, keyword: &str) -> Option<&'a str> {
    let lower = value.to_ascii_lowercase();
    let kw = format!("{} ", keyword.to_ascii_lowercase());
    if lower.starts_with(&kw) {
        Some(&value[kw.len()..])
    } else {
        None
    }
}

fn scalar_to_text(value: &ScalarValue) -> String {
    match value {
        ScalarValue::String(value) => value.clone(),
        ScalarValue::I64(value) => value.to_string(),
        ScalarValue::F64(value) => value.to_string(),
        ScalarValue::Bool(value) => value.to_string(),
    }
}

// ----------------------------------------------------------------------------
// Postgres wire codec (server side: parse frontend, emit backend). Self
// contained so this module does not touch the lib.rs server while it is being
// co-edited; consolidate into one codec once that lane settles.
// ----------------------------------------------------------------------------

/// A parsed (Parse-message) prepared statement: the SQL plus the parameter type
/// OIDs the client declared, echoed back in ParameterDescription.
#[derive(Clone, Default)]
struct PreparedStmt {
    sql: String,
    param_oids: Vec<u32>,
}

/// Format code for parameter `index`: 0 codes -> text; 1 code -> applies to all;
/// otherwise per-parameter.
fn format_code_at(formats: &[i16], index: usize) -> i16 {
    match formats.len() {
        0 => 0,
        1 => formats[0],
        _ => formats.get(index).copied().unwrap_or(0),
    }
}

/// Render a bound parameter as a SQL literal for substitution into the query
/// text, honoring the wire format (text=0/binary=1) and the param type OID.
fn render_param(value: Option<&[u8]>, format: i16, oid: u32) -> String {
    let Some(bytes) = value else {
        return "NULL".to_string();
    };
    if format == 1 {
        if oid == Type::INT8.oid() && bytes.len() == 8 {
            return i64::from_be_bytes(bytes.try_into().unwrap()).to_string();
        }
        if oid == Type::INT4.oid() && bytes.len() == 4 {
            return i32::from_be_bytes(bytes.try_into().unwrap()).to_string();
        }
        if oid == Type::FLOAT8.oid() && bytes.len() == 8 {
            return f64::from_be_bytes(bytes.try_into().unwrap()).to_string();
        }
        if oid == Type::BOOL.oid() && bytes.len() == 1 {
            return if bytes[0] != 0 { "TRUE" } else { "FALSE" }.to_string();
        }
        return sql_literal_string(&String::from_utf8_lossy(bytes));
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    // Never splice raw client text into the SQL on the strength of a CLAIMED
    // numeric OID. Validate it actually parses as that type and re-serialize the
    // parsed value; on failure fall back to a quoted string literal. Otherwise a
    // client could declare $1 as INT8 and send text like "0 AND x > 1",
    // injecting a predicate fragment into the parameterized path.
    if oid == Type::INT8.oid() || oid == Type::INT4.oid() {
        return match trimmed.parse::<i64>() {
            Ok(value) => value.to_string(),
            Err(_) => sql_literal_string(&text),
        };
    }
    if oid == Type::FLOAT8.oid() || oid == Type::NUMERIC.oid() {
        return match trimmed.parse::<f64>() {
            Ok(value) if value.is_finite() => value.to_string(),
            _ => sql_literal_string(&text),
        };
    }
    if oid == Type::BOOL.oid() {
        return if matches!(trimmed, "t" | "true" | "TRUE" | "1") {
            "TRUE"
        } else {
            "FALSE"
        }
        .to_string();
    }
    sql_literal_string(&text)
}

fn sql_literal_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Replace `$N` placeholders OUTSIDE single-quoted strings with `literals[N-1]`.
fn substitute_placeholders(sql: &str, literals: &[String]) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if in_string {
            out.push(ch);
            if ch == '\'' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if ch == '\'' {
            in_string = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '$' && index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let number: usize = sql[index + 1..end].parse().unwrap_or(0);
            if number >= 1 && number <= literals.len() {
                out.push_str(&literals[number - 1]);
            } else {
                out.push_str(&sql[index..end]);
            }
            index = end;
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

/// Statement-level Describe cannot run a parameterized query without values. To
/// recover the column shape (names + types), strip the WHERE/ORDER/LIMIT (which
/// carry the placeholders) and run only the parameter-free projection + FROM.
/// Returns None when the SQL has no placeholders (run it as-is for full typing).
fn probe_sql_for_describe(sql: &str) -> Option<String> {
    if !sql.contains('$') {
        return None;
    }
    let trimmed = sql.trim().trim_end_matches(';');
    let statements = Parser::parse_sql(&GenericDialect {}, trimmed).ok()?;
    let mut statement = statements.into_iter().next()?;
    let Statement::Query(query) = &mut statement else {
        return None;
    };
    if let SetExpr::Select(select) = query.body.as_mut() {
        select.selection = None;
    }
    query.order_by = None;
    query.limit_clause = None;
    Some(query.to_string())
}

fn max_placeholder(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut max = 0;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if in_string {
            if ch == '\'' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if ch == '\'' {
            in_string = true;
            index += 1;
            continue;
        }
        if ch == '$' && index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if let Ok(number) = sql[index + 1..end].parse::<usize>() {
                max = max.max(number);
            }
            index = end;
            continue;
        }
        index += 1;
    }
    max
}

#[derive(Clone, Debug, PartialEq)]
enum FrontendMessage {
    Query(String),
    Parse {
        name: String,
        query: String,
        param_oids: Vec<u32>,
    },
    Bind {
        portal: String,
        statement: String,
        param_formats: Vec<i16>,
        param_values: Vec<Option<Vec<u8>>>,
        result_formats: Vec<i16>,
    },
    Describe { target: u8, name: String },
    Execute { portal: String },
    Sync,
    Flush,
    Close,
    Terminate,
    Password(String),
}

/// Returns Ok(false) if the client closed before completing startup.
fn read_startup(stream: &mut TcpStream) -> std::io::Result<bool> {
    loop {
        let len = match read_i32_opt(stream)? {
            Some(len) => len,
            None => return Ok(false),
        };
        if !(8..=MAX_MESSAGE_LEN).contains(&len) {
            return Err(invalid_data("startup packet length out of range"));
        }
        let len = len as usize;
        let mut body = vec![0_u8; len - 4];
        stream.read_exact(&mut body)?;
        let code = i32::from_be_bytes(body[..4].try_into().unwrap());
        if code == SSL_REQUEST || code == GSS_REQUEST {
            // Decline encryption negotiation; the client retries in cleartext.
            stream.write_all(b"N")?;
            stream.flush()?;
            continue;
        }
        if code != PROTOCOL_VERSION_3 {
            return Err(invalid_data("unsupported Postgres protocol version"));
        }
        return Ok(true);
    }
}

fn read_frontend_message(stream: &mut TcpStream) -> std::io::Result<Option<FrontendMessage>> {
    let mut tag = [0_u8; 1];
    match stream.read_exact(&mut tag) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = read_i32(stream)?;
    if !(4..=MAX_MESSAGE_LEN).contains(&len) {
        return Err(invalid_data("frontend message length out of range"));
    }
    let len = len as usize;
    let mut body = vec![0_u8; len - 4];
    stream.read_exact(&mut body)?;
    Ok(Some(parse_frontend_message(tag[0], &body)?))
}

fn parse_frontend_message(tag: u8, body: &[u8]) -> std::io::Result<FrontendMessage> {
    Ok(match tag {
        b'Q' => {
            let mut cursor = 0;
            FrontendMessage::Query(read_cstr_at(body, &mut cursor)?)
        }
        b'P' => {
            let mut cursor = 0;
            let name = read_cstr_at(body, &mut cursor)?;
            let query = read_cstr_at(body, &mut cursor)?;
            // Parameter type OIDs the client declared.
            let count = read_i16_at(body, &mut cursor)?;
            let mut param_oids = Vec::new();
            for _ in 0..count.max(0) {
                param_oids.push(read_i32_at(body, &mut cursor)? as u32);
            }
            FrontendMessage::Parse {
                name,
                query,
                param_oids,
            }
        }
        b'B' => {
            let mut cursor = 0;
            let portal = read_cstr_at(body, &mut cursor)?;
            let statement = read_cstr_at(body, &mut cursor)?;
            // Parameter format codes.
            let format_count = read_i16_at(body, &mut cursor)?;
            let mut param_formats = Vec::new();
            for _ in 0..format_count.max(0) {
                param_formats.push(read_i16_at(body, &mut cursor)?);
            }
            // Parameter values (-1 length sentinel = NULL).
            let value_count = read_i16_at(body, &mut cursor)?;
            let mut param_values = Vec::new();
            for _ in 0..value_count.max(0) {
                let len = read_i32_at(body, &mut cursor)?;
                if len < 0 {
                    param_values.push(None);
                } else {
                    let len = len as usize;
                    if cursor + len > body.len() {
                        return Err(invalid_data("Bind parameter value overruns message"));
                    }
                    param_values.push(Some(body[cursor..cursor + len].to_vec()));
                    cursor += len;
                }
            }
            // Result format codes (honored at Execute time).
            let result_count = read_i16_at(body, &mut cursor)?;
            let mut result_formats = Vec::new();
            for _ in 0..result_count.max(0) {
                result_formats.push(read_i16_at(body, &mut cursor)?);
            }
            FrontendMessage::Bind {
                portal,
                statement,
                param_formats,
                param_values,
                result_formats,
            }
        }
        b'D' => {
            let target = *body.first().ok_or_else(|| invalid_data("empty Describe"))?;
            let mut cursor = 1;
            let name = read_cstr_at(body, &mut cursor)?;
            FrontendMessage::Describe { target, name }
        }
        b'E' => {
            let mut cursor = 0;
            let portal = read_cstr_at(body, &mut cursor)?;
            FrontendMessage::Execute { portal }
        }
        b'S' => FrontendMessage::Sync,
        b'H' => FrontendMessage::Flush,
        b'C' => FrontendMessage::Close,
        b'X' => FrontendMessage::Terminate,
        b'p' => {
            let mut cursor = 0;
            FrontendMessage::Password(read_cstr_at(body, &mut cursor)?)
        }
        other => {
            return Err(invalid_data(format!(
                "unsupported frontend message tag {}",
                other as char
            )))
        }
    })
}

fn startup_response() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(authentication_ok());
    out.extend(parameter_status("server_version", "16.0 (rustyred-pg-wire)"));
    out.extend(parameter_status("server_encoding", "UTF8"));
    out.extend(parameter_status("client_encoding", "UTF8"));
    out.extend(parameter_status("DateStyle", "ISO, MDY"));
    out.extend(parameter_status("integer_datetimes", "on"));
    out.extend(parameter_status("standard_conforming_strings", "on"));
    out.extend(backend_key_data(1001, 4242));
    out.extend(ready_for_query());
    out
}

fn encode_query_result(result: &PgQueryResult) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(row_description(&result.columns));
    for row in &result.rows {
        out.extend(data_row(row));
    }
    out.extend(command_complete(&result.command_tag));
    out
}

fn authentication_ok() -> Vec<u8> {
    message(b'R', &0_i32.to_be_bytes())
}

fn parameter_status(name: &str, value: &str) -> Vec<u8> {
    let mut body = Vec::new();
    write_cstr(&mut body, name);
    write_cstr(&mut body, value);
    message(b'S', &body)
}

fn backend_key_data(pid: i32, secret: i32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(pid.to_be_bytes());
    body.extend(secret.to_be_bytes());
    message(b'K', &body)
}

fn ready_for_query() -> Vec<u8> {
    message(b'Z', b"I")
}

fn parse_complete() -> Vec<u8> {
    message(b'1', &[])
}

fn bind_complete() -> Vec<u8> {
    message(b'2', &[])
}

fn close_complete() -> Vec<u8> {
    message(b'3', &[])
}

fn no_data() -> Vec<u8> {
    message(b'n', &[])
}

fn empty_query_response() -> Vec<u8> {
    message(b'I', &[])
}

fn parameter_description(oids: &[u32]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend((oids.len() as i16).to_be_bytes());
    for oid in oids {
        body.extend(oid.to_be_bytes());
    }
    message(b't', &body)
}

fn row_description(columns: &[PgColumn]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend((columns.len() as i16).to_be_bytes());
    for column in columns {
        write_cstr(&mut body, &column.name);
        body.extend(0_u32.to_be_bytes()); // table oid
        body.extend(0_i16.to_be_bytes()); // attribute number
        body.extend(column.type_oid.to_be_bytes());
        body.extend(column.type_size.to_be_bytes());
        body.extend((-1_i32).to_be_bytes()); // type modifier
        body.extend(0_i16.to_be_bytes()); // text format
    }
    message(b'T', &body)
}

fn data_row(values: &[Option<String>]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend((values.len() as i16).to_be_bytes());
    for value in values {
        match value {
            Some(value) => {
                body.extend((value.len() as i32).to_be_bytes());
                body.extend(value.as_bytes());
            }
            None => body.extend((-1_i32).to_be_bytes()),
        }
    }
    message(b'D', &body)
}

/// Extended-protocol DataRow that honors the per-column result format the client
/// requested in Bind. psql requests text; tokio-postgres requests binary.
fn data_row_typed(values: &[Option<String>], columns: &[PgColumn], formats: &[i16]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend((values.len() as i16).to_be_bytes());
    for (index, value) in values.iter().enumerate() {
        match value {
            None => body.extend((-1_i32).to_be_bytes()),
            Some(text) => {
                let encoded = if result_format_is_binary(formats, index) {
                    let oid = columns
                        .get(index)
                        .map(|column| column.type_oid)
                        .unwrap_or_else(|| Type::TEXT.oid());
                    encode_binary(text, oid)
                } else {
                    text.as_bytes().to_vec()
                };
                body.extend((encoded.len() as i32).to_be_bytes());
                body.extend(encoded);
            }
        }
    }
    message(b'D', &body)
}

fn result_format_is_binary(formats: &[i16], index: usize) -> bool {
    match formats.len() {
        0 => false,           // no codes: text for every column
        1 => formats[0] == 1, // one code: applies to all columns
        _ => formats.get(index).copied().unwrap_or(0) == 1,
    }
}

/// Encode a text-rendered value as the Postgres binary wire form for its type.
fn encode_binary(text: &str, oid: u32) -> Vec<u8> {
    if oid == Type::INT8.oid() {
        if let Ok(value) = text.parse::<i64>() {
            return value.to_be_bytes().to_vec();
        }
    } else if oid == Type::FLOAT8.oid() {
        if let Ok(value) = text.parse::<f64>() {
            return value.to_be_bytes().to_vec();
        }
    } else if oid == Type::BOOL.oid() {
        return vec![u8::from(text == "true")];
    }
    // text/varchar and any fallback: binary form is the UTF-8 bytes.
    text.as_bytes().to_vec()
}

fn command_complete(tag: &str) -> Vec<u8> {
    let mut body = Vec::new();
    write_cstr(&mut body, tag);
    message(b'C', &body)
}

fn error_response(error: &PgError) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(b'S');
    write_cstr(&mut body, "ERROR");
    body.push(b'V');
    write_cstr(&mut body, "ERROR");
    body.push(b'C');
    write_cstr(&mut body, &error.sqlstate);
    body.push(b'M');
    write_cstr(&mut body, &error.message);
    body.push(0);
    message(b'E', &body)
}

fn message(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(tag);
    out.extend(((body.len() + 4) as i32).to_be_bytes());
    out.extend(body);
    out
}

fn write_cstr(out: &mut Vec<u8>, value: &str) {
    out.extend(value.as_bytes());
    out.push(0);
}

fn read_i32(stream: &mut TcpStream) -> std::io::Result<i32> {
    let mut bytes = [0_u8; 4];
    stream.read_exact(&mut bytes)?;
    Ok(i32::from_be_bytes(bytes))
}

fn read_i32_opt(stream: &mut TcpStream) -> std::io::Result<Option<i32>> {
    let mut bytes = [0_u8; 4];
    match stream.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(i32::from_be_bytes(bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_i16_at(bytes: &[u8], cursor: &mut usize) -> std::io::Result<i16> {
    if *cursor + 2 > bytes.len() {
        return Err(invalid_data("short read for i16"));
    }
    let value = i16::from_be_bytes([bytes[*cursor], bytes[*cursor + 1]]);
    *cursor += 2;
    Ok(value)
}

fn read_i32_at(bytes: &[u8], cursor: &mut usize) -> std::io::Result<i32> {
    if *cursor + 4 > bytes.len() {
        return Err(invalid_data("short read for i32"));
    }
    let value = i32::from_be_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(value)
}

fn read_cstr_at(bytes: &[u8], cursor: &mut usize) -> std::io::Result<String> {
    let Some(end) = bytes[*cursor..].iter().position(|byte| *byte == 0) else {
        return Err(invalid_data("missing C string terminator"));
    };
    let start = *cursor;
    let end_abs = start + end;
    *cursor = end_abs + 1;
    Ok(String::from_utf8_lossy(&bytes[start..end_abs]).to_string())
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

/// Convenience for the binary and tests: a `RelationalStore` seeded with small
/// native views (memory + epistemic) so the wire surface has something to serve.
pub fn demo_native_store() -> RelationalStore {
    use rustyred_thg_core::RelationalRow;
    let mut store = RelationalStore::new();
    let memory_rows = [
        ("mem:1", "planning", 1000_i64),
        ("mem:2", "review", 2000),
        ("mem:3", "planning", 3000),
    ];
    for (id, topic, ts) in memory_rows {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::String(id.to_string()));
        values.insert("topic".to_string(), ScalarValue::String(topic.to_string()));
        values.insert("created_ms".to_string(), ScalarValue::I64(ts));
        let _ = store.upsert_row(RelationalRow::new("memory", id, values));
    }
    let epistemic_rows = [("ep:1", "mem:1", "supports"), ("ep:2", "mem:3", "undercuts")];
    for (id, content_id, stance) in epistemic_rows {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::String(id.to_string()));
        values.insert("content_id".to_string(), ScalarValue::String(content_id.to_string()));
        values.insert("stance".to_string(), ScalarValue::String(stance.to_string()));
        let _ = store.upsert_row(RelationalRow::new("epistemic", id, values));
    }
    store
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> RelationalStore {
        demo_native_store()
    }

    #[test]
    fn lowers_scalar_select_and_infers_text_oid() {
        let result =
            execute_native_sql("SELECT id, topic FROM memory WHERE topic = 'planning'", &store())
                .unwrap();
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].type_oid, Type::TEXT.oid());
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn infers_int8_oid_for_integer_column() {
        let result = execute_native_sql("SELECT created_ms FROM memory", &store()).unwrap();
        assert_eq!(result.columns[0].type_oid, Type::INT8.oid());
    }

    #[test]
    fn time_range_routes_to_time_series_index() {
        let result = execute_native_sql(
            "EXPLAIN SELECT id FROM memory WHERE time_range(created_ms, 500, 2500)",
            &store(),
        )
        .unwrap();
        let plan = result
            .rows
            .iter()
            .filter_map(|row| row[0].clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan.contains("time_series"), "plan was: {plan}");
        assert!(plan.contains("full_relation_scans=0"), "plan was: {plan}");
    }

    #[test]
    fn join_lowers_to_planner_hash_join() {
        let result = execute_native_sql(
            "SELECT m.topic, e.stance FROM memory m JOIN epistemic e ON m.id = e.content_id",
            &store(),
        )
        .unwrap();
        assert!(!result.rows.is_empty());
        let trace = result.trace.unwrap();
        assert_eq!(trace["join_algorithm"], "hash_join");
    }

    #[test]
    fn group_by_count_aggregates() {
        let result = execute_native_sql(
            "SELECT topic, count(*) FROM memory GROUP BY topic",
            &store(),
        )
        .unwrap();
        // two topics: planning (2) and review (1)
        assert_eq!(result.rows.len(), 2);
        let counts: Vec<String> = result.rows.iter().filter_map(|row| row[1].clone()).collect();
        assert!(counts.contains(&"2".to_string()));
        assert!(counts.contains(&"1".to_string()));
    }

    #[test]
    fn order_by_and_limit_apply() {
        let result =
            execute_native_sql("SELECT id FROM memory ORDER BY created_ms DESC LIMIT 1", &store())
                .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], Some("mem:3".to_string()));
    }

    #[test]
    fn rejects_non_select() {
        let error = execute_native_sql("DELETE FROM memory", &store()).unwrap_err();
        assert_eq!(error.sqlstate, "0A000");
    }

    // --- regression tests for the adversarial-review findings ---

    #[test]
    fn count_distinct_dedupes() {
        // count(DISTINCT topic) = 2 (planning, review); count(topic) = 3.
        let distinct =
            execute_native_sql("SELECT count(DISTINCT topic) FROM memory", &store()).unwrap();
        assert_eq!(distinct.rows[0][0], Some("2".to_string()));
        let total = execute_native_sql("SELECT count(topic) FROM memory", &store()).unwrap();
        assert_eq!(total.rows[0][0], Some("3".to_string()));
    }

    #[test]
    fn select_star_join_keeps_both_id_columns() {
        // m.id and e.id both render as display "id"; positional shaping must keep
        // them distinct rather than collapsing on the shared name.
        let result = execute_native_sql(
            "SELECT * FROM memory m JOIN epistemic e ON m.id = e.content_id",
            &store(),
        )
        .unwrap();
        let id_columns = result.columns.iter().filter(|c| c.name == "id").count();
        assert_eq!(id_columns, 2, "the join exposes two id columns");
        let values: Vec<String> = result.rows[0].iter().filter_map(|v| v.clone()).collect();
        assert!(values.iter().any(|v| v.starts_with("mem:")), "memory id kept: {values:?}");
        assert!(values.iter().any(|v| v.starts_with("ep:")), "epistemic id kept: {values:?}");
    }

    #[test]
    fn rejects_unindexed_modality_predicates() {
        for sql in [
            "SELECT id FROM memory WHERE text_match(topic, 'planning')",
            "SELECT id FROM memory WHERE knn(topic, '[0.1]', 2)",
            "SELECT id FROM memory WHERE geo_within(topic, 0, 0, 1, 1)",
        ] {
            let error = execute_native_sql(sql, &store()).unwrap_err();
            assert_eq!(error.sqlstate, "0A000", "must refuse unindexed modality: {sql}");
        }
    }

    #[test]
    fn substitutes_params_outside_string_literals() {
        let sql = "SELECT id FROM memory WHERE topic = $1 AND note = 'cost is $1'";
        let out = substitute_placeholders(sql, &["'planning'".to_string()]);
        assert_eq!(
            out,
            "SELECT id FROM memory WHERE topic = 'planning' AND note = 'cost is $1'"
        );
    }

    #[test]
    fn renders_params_by_format_and_type() {
        assert_eq!(render_param(Some(&1000_i64.to_be_bytes()), 1, Type::INT8.oid()), "1000");
        assert_eq!(render_param(Some(b"a'b"), 0, Type::TEXT.oid()), "'a''b'");
        assert_eq!(render_param(Some(b"1000"), 0, Type::INT8.oid()), "1000");
        assert_eq!(render_param(None, 1, Type::INT8.oid()), "NULL");
    }

    #[test]
    fn numeric_param_text_is_validated_not_spliced_raw() {
        // A client declaring $1 as INT8 but sending a text injection attempt must
        // NOT be spliced raw; it falls back to a quoted (harmless) string literal.
        assert_eq!(
            render_param(Some(b"0 AND created_ms > 0"), 0, Type::INT8.oid()),
            "'0 AND created_ms > 0'"
        );
        // a genuine numeric text value is re-serialized as a bare number
        assert_eq!(render_param(Some(b"  42 "), 0, Type::INT8.oid()), "42");
        assert_eq!(render_param(Some(b"1.5x"), 0, Type::FLOAT8.oid()), "'1.5x'");
    }
}
