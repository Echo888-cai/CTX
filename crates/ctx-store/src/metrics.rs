//! Aggregations for the local dashboard. Kernel store stays unchanged.

use rusqlite::params;

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
        let conn = self.reader();
        let row = if let Some(model) = model {
            conn.query_row(
                "SELECT
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0),
                    COALESCE(SUM(o.avoided_tokens), 0),
                    COALESCE(SUM(o.refetched_tokens), 0)
                 FROM observations o
                 LEFT JOIN sessions s ON s.id = o.session_id
                 WHERE o.created_at >= ?1
                   AND CASE WHEN ?2 = '__unknown__'
                            THEN COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ''
                            ELSE COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ?2
                       END",
                params![since, model],
                map_totals,
            )
        } else {
            conn.query_row(
                "SELECT
                    COALESCE(SUM(raw_tokens), 0),
                    COALESCE(SUM(delivered_tokens), 0),
                    COALESCE(SUM(avoided_tokens), 0),
                    COALESCE(SUM(refetched_tokens), 0)
                 FROM observations WHERE created_at >= ?1",
                params![since],
                map_totals,
            )
        };
        row.map_err(Into::into)
    }

    pub fn dashboard_models(&self, since: i64) -> Result<Vec<ModelRow>> {
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
             WHERE o.created_at >= ?1
             GROUP BY 1
             ORDER BY SUM(o.avoided_tokens) DESC",
        )?;
        let rows = stmt.query_map(params![since], |r| {
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
        let conn = self.reader();
        let sql = if model.is_some() {
            "SELECT ((o.created_at + ?2) / ?3) * ?3 - ?2,
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1
               AND CASE WHEN ?4 = '__unknown__'
                        THEN COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ''
                        ELSE COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ?4
                   END
             GROUP BY 1
             ORDER BY 1"
        } else {
            "SELECT ((created_at + ?2) / ?3) * ?3 - ?2,
                    COALESCE(SUM(raw_tokens), 0),
                    COALESCE(SUM(delivered_tokens), 0)
             FROM observations
             WHERE created_at >= ?1
             GROUP BY 1
             ORDER BY 1"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut points = Vec::new();
        if let Some(model) = model {
            let rows = stmt.query_map(params![since, tz, bucket, model], map_point)?;
            for row in rows {
                points.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![since, tz, bucket], map_point)?;
            for row in rows {
                points.push(row?);
            }
        }
        Ok(points)
    }

    pub fn dashboard_by_harness(
        &self,
        since: i64,
        model: Option<&str>,
    ) -> Result<Vec<(String, TokenTotals)>> {
        let conn = self.reader();
        let sql = if model.is_some() {
            "SELECT COALESCE(s.harness, 'unknown'),
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0),
                    COALESCE(SUM(o.avoided_tokens), 0),
                    COALESCE(SUM(o.refetched_tokens), 0)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1
               AND CASE WHEN ?2 = '__unknown__'
                        THEN COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ''
                        ELSE COALESCE(NULLIF(o.model, ''), NULLIF(s.model, ''), '') = ?2
                   END
             GROUP BY 1"
        } else {
            "SELECT COALESCE(s.harness, 'unknown'),
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0),
                    COALESCE(SUM(o.avoided_tokens), 0),
                    COALESCE(SUM(o.refetched_tokens), 0)
             FROM observations o
             LEFT JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1
             GROUP BY 1"
        };
        let mut stmt = conn.prepare(sql)?;
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
        let rows = if let Some(model) = model {
            stmt.query_map(params![since, model], map)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![since], map)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
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
    }
}
