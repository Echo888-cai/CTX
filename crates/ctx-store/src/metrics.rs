//! Aggregations for the local dashboard. Kernel store stays unchanged.

use rusqlite::{params, params_from_iter};

use super::{Result, Store, TokenTotals};

#[derive(Debug, Clone)]
pub struct SeriesPoint {
    pub t: i64,
    pub raw: u64,
    pub delivered: u64,
}

#[derive(Debug, Clone)]
pub struct ModelRow {
    pub id: String,
    pub sessions: u64,
    pub totals: TokenTotals,
    pub source_harnesses: Vec<String>,
}

impl Store {
    pub fn dashboard_totals(&self, since: i64, model: Option<&str>) -> Result<TokenTotals> {
        let owned = model.map(|m| vec![m.to_string()]);
        self.dashboard_totals_for(since, i64::MAX, owned.as_deref())
    }

    pub fn dashboard_totals_for(
        &self,
        since: i64,
        until: i64,
        models: Option<&[String]>,
    ) -> Result<TokenTotals> {
        let conn = self.reader();
        let Some(models) = models.filter(|m| !m.is_empty()) else {
            return conn
                .query_row(
                    "SELECT
                        COALESCE(SUM(raw_tokens), 0),
                        COALESCE(SUM(delivered_tokens), 0),
                        COALESCE(SUM(avoided_tokens), 0),
                        COALESCE(SUM(refetched_tokens), 0)
                     FROM observations WHERE created_at >= ?1 AND created_at <= ?2",
                    params![since, until],
                    map_totals,
                )
                .map_err(Into::into);
        };
        let (filter, binds) = model_filter_clause(models);
        let sql = format!(
            "SELECT
                COALESCE(SUM(o.raw_tokens), 0),
                COALESCE(SUM(o.delivered_tokens), 0),
                COALESCE(SUM(o.avoided_tokens), 0),
                COALESCE(SUM(o.refetched_tokens), 0)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1 AND o.created_at <= ?2
               AND ({filter})"
        );
        conn.query_row(
            &sql,
            params_from_iter(
                [
                    rusqlite::types::Value::from(since),
                    rusqlite::types::Value::from(until),
                ]
                .into_iter()
                .chain(binds.into_iter().map(Into::into)),
            ),
            map_totals,
        )
        .map_err(Into::into)
    }

    pub fn dashboard_models(&self, since: i64) -> Result<Vec<ModelRow>> {
        self.dashboard_models_between(since, i64::MAX)
    }

    pub fn dashboard_models_between(&self, since: i64, until: i64) -> Result<Vec<ModelRow>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '__unknown__'),
                    COUNT(DISTINCT o.session_id),
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0),
                    COALESCE(SUM(o.avoided_tokens), 0),
                    COALESCE(SUM(o.refetched_tokens), 0),
                    GROUP_CONCAT(DISTINCT s.harness)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1 AND o.created_at <= ?2
             GROUP BY 1
             ORDER BY SUM(o.avoided_tokens) DESC",
        )?;
        let rows = stmt.query_map(params![since, until], |r| {
            Ok(ModelRow {
                id: r.get(0)?,
                sessions: r.get::<_, i64>(1)? as u64,
                totals: TokenTotals {
                    raw: r.get::<_, i64>(2)? as u64,
                    delivered: r.get::<_, i64>(3)? as u64,
                    avoided: r.get::<_, i64>(4)? as u64,
                    refetched: r.get::<_, i64>(5).unwrap_or(0) as u64,
                },
                source_harnesses: r
                    .get::<_, String>(6)?
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn dashboard_series(
        &self,
        since: i64,
        bucket: i64,
        tz: i64,
        model: Option<&str>,
    ) -> Result<Vec<SeriesPoint>> {
        let owned = model.map(|m| vec![m.to_string()]);
        self.dashboard_series_for(since, i64::MAX, bucket, tz, owned.as_deref())
    }

    pub fn dashboard_series_for(
        &self,
        since: i64,
        until: i64,
        bucket: i64,
        tz: i64,
        models: Option<&[String]>,
    ) -> Result<Vec<SeriesPoint>> {
        let conn = self.reader();
        let mut points = Vec::new();
        let Some(models) = models.filter(|m| !m.is_empty()) else {
            let mut stmt = conn.prepare(
                "SELECT ((created_at + ?2) / ?3) * ?3 - ?2,
                        COALESCE(SUM(raw_tokens), 0),
                        COALESCE(SUM(delivered_tokens), 0)
                 FROM observations
                 WHERE created_at >= ?1 AND created_at <= ?4
                 GROUP BY 1
                 ORDER BY 1",
            )?;
            for row in stmt.query_map(params![since, tz, bucket, until], map_point)? {
                points.push(row?);
            }
            return Ok(points);
        };
        let (filter, binds) = model_filter_clause(models);
        let sql = format!(
            "SELECT ((o.created_at + ?2) / ?3) * ?3 - ?2,
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1 AND o.created_at <= ?4
               AND ({filter})
             GROUP BY 1
             ORDER BY 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let iter = params_from_iter(
            [
                rusqlite::types::Value::from(since),
                rusqlite::types::Value::from(tz),
                rusqlite::types::Value::from(bucket),
                rusqlite::types::Value::from(until),
            ]
            .into_iter()
            .chain(binds.into_iter().map(Into::into)),
        );
        for row in stmt.query_map(iter, map_point)? {
            points.push(row?);
        }
        Ok(points)
    }

    pub fn dashboard_by_harness(
        &self,
        since: i64,
        model: Option<&str>,
    ) -> Result<Vec<(String, TokenTotals)>> {
        let owned = model.map(|m| vec![m.to_string()]);
        self.dashboard_by_harness_for(since, i64::MAX, owned.as_deref())
    }

    pub fn dashboard_by_harness_for(
        &self,
        since: i64,
        until: i64,
        models: Option<&[String]>,
    ) -> Result<Vec<(String, TokenTotals)>> {
        let conn = self.reader();
        let map = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get::<_, String>(0)?,
                TokenTotals {
                    raw: r.get::<_, i64>(1)? as u64,
                    delivered: r.get::<_, i64>(2)? as u64,
                    avoided: r.get::<_, i64>(3)? as u64,
                    refetched: r.get::<_, i64>(4).unwrap_or(0) as u64,
                },
            ))
        };
        let Some(models) = models.filter(|m| !m.is_empty()) else {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(s.harness, 'unknown'),
                        COALESCE(SUM(o.raw_tokens), 0),
                        COALESCE(SUM(o.delivered_tokens), 0),
                        COALESCE(SUM(o.avoided_tokens), 0),
                        COALESCE(SUM(o.refetched_tokens), 0)
                 FROM observations o
                 LEFT JOIN sessions s ON s.id = o.session_id
                 WHERE o.created_at >= ?1 AND o.created_at <= ?2
                 GROUP BY 1",
            )?;
            return stmt
                .query_map(params![since, until], map)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into);
        };
        let (filter, binds) = model_filter_clause(models);
        let sql = format!(
            "SELECT COALESCE(s.harness, 'unknown'),
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0),
                    COALESCE(SUM(o.avoided_tokens), 0),
                    COALESCE(SUM(o.refetched_tokens), 0)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1 AND o.created_at <= ?2
               AND ({filter})
             GROUP BY 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let iter = params_from_iter(
            [
                rusqlite::types::Value::from(since),
                rusqlite::types::Value::from(until),
            ]
            .into_iter()
            .chain(binds.into_iter().map(Into::into)),
        );
        let rows = stmt
            .query_map(iter, map)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

fn model_filter_clause(models: &[String]) -> (String, Vec<String>) {
    let include_unknown = models.iter().any(|m| m == "__unknown__");
    let named: Vec<String> = models
        .iter()
        .filter(|m| m.as_str() != "__unknown__")
        .cloned()
        .collect();
    match (include_unknown, named.is_empty()) {
        (true, true) => (
            "COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ''".into(),
            Vec::new(),
        ),
        (true, false) => {
            let placeholders = vec!["?"; named.len()].join(", ");
            (
                format!(
                    "COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = '' \
                     OR COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') IN ({placeholders})"
                ),
                named,
            )
        }
        (false, _) => {
            let placeholders = vec!["?"; named.len().max(1)].join(", ");
            if named.is_empty() {
                ("0".into(), Vec::new())
            } else {
                (
                    format!(
                        "COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') IN ({placeholders})"
                    ),
                    named,
                )
            }
        }
    }
}

fn map_totals(r: &rusqlite::Row<'_>) -> rusqlite::Result<TokenTotals> {
    Ok(TokenTotals {
        raw: r.get::<_, i64>(0)? as u64,
        delivered: r.get::<_, i64>(1)? as u64,
        avoided: r.get::<_, i64>(2)? as u64,
        refetched: r.get::<_, i64>(3).unwrap_or(0) as u64,
    })
}

fn map_point(r: &rusqlite::Row<'_>) -> rusqlite::Result<SeriesPoint> {
    Ok(SeriesPoint {
        t: r.get(0)?,
        raw: r.get::<_, i64>(1)? as u64,
        delivered: r.get::<_, i64>(2)? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CtxPaths, NewObservation};

    fn add_observation(store: &Store, session: &str, raw: u32, delivered: u32) {
        store
            .insert_observation(NewObservation {
                session_id: session.into(),
                model: String::new(),
                event_type: "tool_output".into(),
                tool_type: Some("shell".into()),
                tool_name: Some("Bash".into()),
                uri: None,
                content_hash: format!("hash-{session}"),
                raw_tokens: raw,
                delivered_tokens: delivered,
                avoided_tokens: raw - delivered,
                optimizer: Some("test".into()),
                reasons: serde_json::json!([]),
                referenced: false,
                source_path: None,
                dedup_key: String::new(),
                shadow: false,
            })
            .unwrap();
    }

    #[test]
    fn dashboard_groups_and_filters_by_model_not_harness() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        for (id, model) in [
            ("gpt-session", "gpt-5"),
            ("claude-session", "claude-sonnet-4-6"),
            ("legacy-session", ""),
        ] {
            store.ensure_session(id, "cursor", None).unwrap();
            store
                .lock()
                .execute(
                    "UPDATE sessions SET model = ?1 WHERE id = ?2",
                    rusqlite::params![model, id],
                )
                .unwrap();
        }
        add_observation(&store, "gpt-session", 100, 40);
        add_observation(&store, "claude-session", 200, 80);
        add_observation(&store, "legacy-session", 50, 20);

        let rows = store.dashboard_models(0).unwrap();
        let ids = rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, vec!["claude-sonnet-4-6", "gpt-5", "__unknown__"]);

        let gpt = store.dashboard_totals(0, Some("gpt-5")).unwrap();
        assert_eq!((gpt.raw, gpt.delivered, gpt.avoided), (100, 40, 60));
        let unknown = store.dashboard_totals(0, Some("__unknown__")).unwrap();
        assert_eq!(
            (unknown.raw, unknown.delivered, unknown.avoided),
            (50, 20, 30)
        );
        let both = store
            .dashboard_totals_for(0, i64::MAX, Some(&["gpt-5".into(), "__unknown__".into()]))
            .unwrap();
        assert_eq!(both.raw, 150);
    }
}
