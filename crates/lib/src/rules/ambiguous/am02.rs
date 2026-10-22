use hashbrown::HashMap;
use sqruff_lib_core::dialects::init::DialectKind;
use sqruff_lib_core::dialects::syntax::{SyntaxKind, SyntaxSet};
use sqruff_lib_core::lint_fix::LintFix;
use sqruff_lib_core::parser::segments::SegmentBuilder;

use crate::core::config::Value;
use crate::core::rules::context::RuleContext;
use crate::core::rules::crawlers::{Crawler, SegmentSeekerCrawler};
use crate::core::rules::{Erased as _, ErasedRule, LintResult, Rule, RuleGroups};

#[derive(Clone, Debug, Default)]
pub struct RuleAM02;

impl Rule for RuleAM02 {
    fn load_from_config(&self, _config: &HashMap<String, Value>) -> Result<ErasedRule, String> {
        Ok(RuleAM02.erased())
    }

    fn name(&self) -> &'static str {
        "ambiguous.union"
    }

    fn description(&self) -> &'static str {
        "Look for UNION keyword not immediately followed by DISTINCT or ALL"
    }

    fn long_description(&self) -> &'static str {
        r#"
**Anti-pattern**

In this example, `UNION DISTINCT` should be preferred over `UNION`, because explicit is better than implicit.


```sql
SELECT a, b FROM table_1
UNION
SELECT a, b FROM table_2
```

**Best practice**

Specify `DISTINCT` or `ALL` after `UNION` (note that `DISTINCT` is the default behavior).

```sql
SELECT a, b FROM table_1
UNION DISTINCT
SELECT a, b FROM table_2
```
"#
    }

    fn groups(&self) -> &'static [RuleGroups] {
        &[RuleGroups::All, RuleGroups::Core, RuleGroups::Ambiguous]
    }

    fn dialect_skip(&self) -> &'static [DialectKind] {
        // Match SQLFluff: only run where UNION DISTINCT is valid syntax
        // (ansi, bigquery, clickhouse, databricks, db2, hive, mysql, redshift,
        // snowflake, trino). Elsewhere the fix would produce invalid SQL.
        &[
            DialectKind::Athena,
            DialectKind::Doris,
            DialectKind::Duckdb,
            DialectKind::Exasol,
            DialectKind::Flink,
            DialectKind::Greenplum,
            DialectKind::Impala,
            DialectKind::Mariadb,
            DialectKind::Materialize,
            DialectKind::Oracle,
            DialectKind::Postgres,
            DialectKind::Sparksql,
            DialectKind::Sqlite,
            DialectKind::Starrocks,
            DialectKind::Teradata,
            DialectKind::Tsql,
            DialectKind::Vertica,
        ]
    }

    fn eval(&self, rule_cx: &RuleContext) -> Vec<LintResult> {
        let raw = rule_cx.segment.raw();
        let raw_upper = raw.to_uppercase();

        if rule_cx.segment.raw().contains("union")
            && !(raw_upper.contains("ALL") || raw_upper.contains("DISTINCT"))
        {
            let edits = vec![
                SegmentBuilder::keyword(rule_cx.tables.next_id(), "union"),
                SegmentBuilder::whitespace(rule_cx.tables.next_id(), " "),
                SegmentBuilder::keyword(rule_cx.tables.next_id(), "distinct"),
            ];

            let segments = rule_cx.segment.clone();
            let fixes = vec![LintFix::replace(
                rule_cx.segment.segments()[0].clone(),
                edits,
                None,
            )];

            return vec![LintResult::new(Some(segments), fixes, None, None)];
        } else if raw_upper.contains("UNION")
            && !(raw_upper.contains("ALL") || raw_upper.contains("DISTINCT"))
        {
            let edits = vec![
                SegmentBuilder::keyword(rule_cx.tables.next_id(), "UNION"),
                SegmentBuilder::whitespace(rule_cx.tables.next_id(), " "),
                SegmentBuilder::keyword(rule_cx.tables.next_id(), "DISTINCT"),
            ];

            let segments = rule_cx.segment.clone();
            let fixes = vec![LintFix::replace(
                rule_cx.segment.segments()[0].clone(),
                edits,
                None,
            )];

            return vec![LintResult::new(Some(segments), fixes, None, None)];
        }

        Vec::new()
    }

    fn is_fix_compatible(&self) -> bool {
        true
    }

    fn crawl_behaviour(&self) -> Crawler {
        SegmentSeekerCrawler::new(const { SyntaxSet::new(&[SyntaxKind::SetOperator]) }).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_for_upstream_dialects() {
        let skipped = RuleAM02.dialect_skip();

        for dialect in [
            DialectKind::Bigquery,
            DialectKind::Clickhouse,
            DialectKind::Databricks,
            DialectKind::Db2,
            DialectKind::Snowflake,
            DialectKind::Trino,
        ] {
            assert!(!skipped.contains(&dialect));
        }
    }

    #[test]
    fn skipped_for_dialects_without_union_distinct() {
        let skipped = RuleAM02.dialect_skip();

        for dialect in [DialectKind::Tsql, DialectKind::Oracle, DialectKind::Sqlite] {
            assert!(skipped.contains(&dialect));
        }
    }
}
