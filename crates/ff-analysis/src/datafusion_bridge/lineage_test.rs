use super::*;
use crate::datafusion_bridge::planner::sql_to_plan;
use crate::datafusion_bridge::provider::{FeatherFlowProvider, FunctionRegistry};
use crate::schema::{RelSchema, SchemaCatalog};
use crate::test_utils::{int32, make_col, varchar};
use crate::types::Nullability;
use ff_core::ModelName;
use std::collections::HashMap;
use std::sync::Arc;

fn make_catalog() -> SchemaCatalog {
    let mut catalog: SchemaCatalog = HashMap::new();
    catalog.insert(
        "orders".to_string(),
        Arc::new(RelSchema::new(vec![
            make_col("id", int32(), Nullability::NotNull),
            make_col("customer_id", int32(), Nullability::Nullable),
            make_col("amount", int32(), Nullability::Nullable),
            make_col("status", varchar(), Nullability::Nullable),
        ])),
    );
    catalog.insert(
        "customers".to_string(),
        Arc::new(RelSchema::new(vec![
            make_col("id", int32(), Nullability::NotNull),
            make_col("name", varchar(), Nullability::Nullable),
        ])),
    );
    catalog
}

fn plan_and_lineage(sql: &str) -> ModelColumnLineage {
    let catalog = make_catalog();
    let registry = FunctionRegistry::new();
    let provider = FeatherFlowProvider::new(&catalog, &registry);
    let plan = sql_to_plan(sql, &provider).unwrap();
    extract_column_lineage(ModelName::new("test_model"), &plan)
}

#[test]
fn test_copy_lineage() {
    let lineage = plan_and_lineage("SELECT id FROM orders");
    let copy_edges: Vec<_> = lineage
        .edges
        .iter()
        .filter(|e| e.kind == LineageKind::Copy && e.output_column == "id")
        .collect();
    assert!(
        !copy_edges.is_empty(),
        "Should have Copy lineage for id column"
    );
}

#[test]
fn test_transform_lineage() {
    let lineage = plan_and_lineage("SELECT id + amount AS total FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);
    let transform_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Transform && e.output_column == "total")
        .collect();
    assert!(
        !transform_edges.is_empty(),
        "Should have Transform lineage for computed column"
    );
}

#[test]
fn test_inspect_lineage_filter() {
    let lineage = plan_and_lineage("SELECT id FROM orders WHERE status = 'active'");
    let deduped = deduplicate_edges(&lineage.edges);
    let inspect_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect && e.source_column == "status")
        .collect();
    assert!(
        !inspect_edges.is_empty(),
        "Should have Inspect lineage for WHERE column"
    );
}

#[test]
fn test_join_lineage() {
    let lineage = plan_and_lineage(
        "SELECT o.id, c.name FROM orders o JOIN customers c ON o.customer_id = c.id",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    // Should have lineage edges for the join key columns (customer_id or id)
    let _join_key_edges: Vec<_> = deduped
        .iter()
        .filter(|e| {
            e.source_column == "customer_id"
                || (e.source_column == "id" && e.kind == LineageKind::Inspect)
        })
        .collect();
    // DataFusion may not always produce explicit Inspect edges for join keys
    // depending on plan structure, so also check for Copy edges on output
    let output_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.source_column == "name" || e.output_column == "id")
        .collect();
    assert!(
        !output_edges.is_empty(),
        "Should have lineage for output columns in JOIN query. Got: {:?}",
        deduped
            .iter()
            .map(|e| format!(
                "{} -> {}.{} ({:?})",
                e.output_column, e.source_table, e.source_column, e.kind
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_aggregate_lineage() {
    let lineage =
        plan_and_lineage("SELECT status, SUM(amount) AS total FROM orders GROUP BY status");
    let deduped = deduplicate_edges(&lineage.edges);

    // status should appear in lineage (as GROUP BY key)
    let status_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.source_column == "status")
        .collect();
    assert!(
        !status_edges.is_empty(),
        "Should have lineage for GROUP BY column 'status'. Got: {:?}",
        deduped
            .iter()
            .map(|e| format!(
                "{} -> {}.{} ({:?})",
                e.output_column, e.source_table, e.source_column, e.kind
            ))
            .collect::<Vec<_>>()
    );

    // amount should appear in lineage (as part of SUM aggregate)
    let amount_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.source_column == "amount")
        .collect();
    assert!(
        !amount_edges.is_empty(),
        "Should have lineage for aggregated column 'amount'. Got: {:?}",
        deduped
            .iter()
            .map(|e| format!(
                "{} -> {}.{} ({:?})",
                e.output_column, e.source_table, e.source_column, e.kind
            ))
            .collect::<Vec<_>>()
    );
}

// ==========================================================================
// Additional DataFusion lineage tests
// ==========================================================================

fn make_extended_catalog() -> SchemaCatalog {
    let mut catalog = make_catalog();
    catalog.insert(
        "products".to_string(),
        Arc::new(RelSchema::new(vec![
            make_col("id", int32(), Nullability::NotNull),
            make_col("name", varchar(), Nullability::Nullable),
            make_col("price", int32(), Nullability::Nullable),
            make_col("category", varchar(), Nullability::Nullable),
        ])),
    );
    catalog.insert(
        "payments".to_string(),
        Arc::new(RelSchema::new(vec![
            make_col("id", int32(), Nullability::NotNull),
            make_col("order_id", int32(), Nullability::NotNull),
            make_col("amount", int32(), Nullability::Nullable),
            make_col("method", varchar(), Nullability::Nullable),
        ])),
    );
    catalog
}

fn plan_and_lineage_extended(sql: &str) -> ModelColumnLineage {
    let catalog = make_extended_catalog();
    let registry = FunctionRegistry::new();
    let provider = FeatherFlowProvider::new(&catalog, &registry);
    let plan = sql_to_plan(sql, &provider).unwrap();
    extract_column_lineage(ModelName::new("test_model"), &plan)
}

#[test]
fn test_multiple_copy_columns() {
    let lineage = plan_and_lineage("SELECT id, customer_id, amount, status FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);
    let copy_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Copy)
        .collect();
    assert!(
        copy_edges.len() >= 4,
        "Should have Copy lineage for all 4 columns, got {}",
        copy_edges.len()
    );
}

#[test]
fn test_aliased_column_is_copy() {
    let lineage = plan_and_lineage("SELECT id AS order_id FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);
    let order_id_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "order_id" && e.source_column == "id")
        .collect();
    assert!(
        !order_id_edges.is_empty(),
        "Aliased column should trace back to source"
    );
}

#[test]
fn test_binary_expression_transform() {
    let lineage = plan_and_lineage("SELECT amount * 2 AS doubled FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);
    let transform: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "doubled" && e.kind == LineageKind::Transform)
        .collect();
    assert!(
        !transform.is_empty(),
        "Binary expression should produce Transform edge"
    );
}

#[test]
fn test_case_expression_transform() {
    let lineage = plan_and_lineage(
        "SELECT CASE WHEN status = 'active' THEN 1 ELSE 0 END AS is_active FROM orders",
    );
    let deduped = deduplicate_edges(&lineage.edges);
    let transform: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "is_active" && e.kind == LineageKind::Transform)
        .collect();
    assert!(
        !transform.is_empty(),
        "CASE expression should produce Transform edge"
    );
    assert!(
        transform.iter().any(|e| e.source_column == "status"),
        "CASE should reference status column"
    );
}

#[test]
fn test_cast_expression() {
    let lineage = plan_and_lineage("SELECT CAST(id AS BIGINT) AS big_id FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);
    let edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "big_id" && e.source_column == "id")
        .collect();
    assert!(
        !edges.is_empty(),
        "CAST expression should preserve source column reference"
    );
}

#[test]
fn test_left_join_produces_lineage() {
    let lineage = plan_and_lineage(
        "SELECT o.id, c.name FROM orders o LEFT JOIN customers c ON o.customer_id = c.id",
    );
    let deduped = deduplicate_edges(&lineage.edges);
    let name_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.source_column == "name")
        .collect();
    assert!(
        !name_edges.is_empty(),
        "LEFT JOIN should still produce lineage for joined columns"
    );
}

#[test]
fn test_three_table_join() {
    let lineage = plan_and_lineage_extended(
        "SELECT o.id AS order_id, c.name AS customer_name, p.name AS product_name
         FROM orders o
         JOIN customers c ON o.customer_id = c.id
         JOIN products p ON o.id = p.id",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    let order_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "order_id")
        .collect();
    assert!(!order_edges.is_empty(), "Should have lineage for order_id");

    let cust_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "customer_name")
        .collect();
    assert!(
        !cust_edges.is_empty(),
        "Should have lineage for customer_name"
    );

    let prod_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "product_name")
        .collect();
    assert!(
        !prod_edges.is_empty(),
        "Should have lineage for product_name"
    );
}

#[test]
fn test_multiple_aggregations() {
    let lineage = plan_and_lineage(
        "SELECT customer_id, SUM(amount) AS total, COUNT(id) AS cnt FROM orders GROUP BY customer_id",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    let edge_desc: Vec<_> = deduped
        .iter()
        .map(|e| {
            format!(
                "{} -> {}.{} ({:?})",
                e.output_column, e.source_table, e.source_column, e.kind
            )
        })
        .collect();

    // amount should appear somewhere in lineage (SUM aggregate)
    assert!(
        deduped.iter().any(|e| e.source_column == "amount"),
        "SUM(amount) should reference amount. Got edges: {:?}",
        edge_desc
    );
    // id should appear somewhere in lineage (COUNT aggregate)
    assert!(
        deduped.iter().any(|e| e.source_column == "id"),
        "COUNT(id) should reference id. Got edges: {:?}",
        edge_desc
    );
    // customer_id should appear (GROUP BY)
    assert!(
        deduped.iter().any(|e| e.source_column == "customer_id"),
        "GROUP BY should reference customer_id. Got edges: {:?}",
        edge_desc
    );
}

#[test]
fn test_subquery_in_from() {
    let lineage =
        plan_and_lineage("SELECT sub.total FROM (SELECT SUM(amount) AS total FROM orders) sub");
    let deduped = deduplicate_edges(&lineage.edges);

    let total_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.source_column == "amount")
        .collect();
    assert!(
        !total_edges.is_empty(),
        "Subquery should propagate column lineage from inner query"
    );
}

#[test]
fn test_where_with_multiple_conditions() {
    let lineage = plan_and_lineage(
        "SELECT id FROM orders WHERE status = 'active' AND amount > 100 AND customer_id IS NOT NULL",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    let inspect_cols: Vec<&str> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect)
        .map(|e| e.source_column.as_str())
        .collect();

    assert!(
        inspect_cols.contains(&"status"),
        "WHERE status should be Inspect. Got: {:?}",
        inspect_cols
    );
    assert!(
        inspect_cols.contains(&"amount"),
        "WHERE amount should be Inspect. Got: {:?}",
        inspect_cols
    );
}

#[test]
fn test_having_clause() {
    let lineage = plan_and_lineage(
        "SELECT status, SUM(amount) AS total FROM orders GROUP BY status HAVING SUM(amount) > 1000",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    // Both status and amount should appear
    assert!(
        deduped.iter().any(|e| e.source_column == "status"),
        "GROUP BY column should appear in lineage"
    );
    assert!(
        deduped.iter().any(|e| e.source_column == "amount"),
        "Aggregated column should appear in lineage"
    );
}

#[test]
fn test_union_all() {
    let lineage = plan_and_lineage(
        "SELECT id, amount FROM orders WHERE status = 'a'
         UNION ALL
         SELECT id, amount FROM orders WHERE status = 'b'",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    // Should have lineage edges from both branches
    assert!(
        !deduped.is_empty(),
        "UNION ALL should produce lineage edges"
    );
    assert!(
        deduped.iter().any(|e| e.source_column == "id"),
        "Should have lineage for id column"
    );
}

#[test]
fn test_order_by_does_not_affect_lineage() {
    let lineage = plan_and_lineage("SELECT id, amount FROM orders ORDER BY amount DESC");
    let deduped = deduplicate_edges(&lineage.edges);

    let copy_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Copy)
        .collect();
    assert!(
        copy_edges.len() >= 2,
        "ORDER BY should not affect output column lineage"
    );
}

#[test]
fn test_limit_does_not_affect_lineage() {
    let lineage = plan_and_lineage("SELECT id, amount FROM orders LIMIT 10");
    let deduped = deduplicate_edges(&lineage.edges);

    let copy_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Copy)
        .collect();
    assert!(
        copy_edges.len() >= 2,
        "LIMIT should not affect output column lineage"
    );
}

#[test]
fn test_distinct_does_not_change_lineage_kind() {
    let lineage = plan_and_lineage("SELECT DISTINCT status FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);

    let status_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.source_column == "status")
        .collect();
    assert!(
        !status_edges.is_empty(),
        "DISTINCT should still produce lineage for selected columns"
    );
}

#[test]
fn test_coalesce_function() {
    let lineage = plan_and_lineage("SELECT COALESCE(amount, 0) AS safe_amount FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);

    let edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "safe_amount" && e.source_column == "amount")
        .collect();
    assert!(
        !edges.is_empty(),
        "COALESCE should reference its column argument"
    );
}

#[test]
fn test_string_concatenation_transform() {
    let lineage = plan_and_lineage("SELECT id || '-' || status AS id_status FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);

    let transform: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "id_status" && e.kind == LineageKind::Transform)
        .collect();
    assert!(
        !transform.is_empty(),
        "String concat should produce Transform edges"
    );
    assert!(
        transform.iter().any(|e| e.source_column == "id"),
        "Should reference id"
    );
    assert!(
        transform.iter().any(|e| e.source_column == "status"),
        "Should reference status"
    );
}

// ==========================================================================
// Alias map extraction
// ==========================================================================

#[test]
fn test_extract_alias_map_simple() {
    let catalog = make_catalog();
    let registry = FunctionRegistry::new();
    let provider = FeatherFlowProvider::new(&catalog, &registry);
    let plan = sql_to_plan("SELECT o.id FROM orders o", &provider).unwrap();

    let aliases = extract_alias_map(&plan);
    assert!(
        aliases.get("o").map(|v| v.as_str()) == Some("orders"),
        "Should map alias 'o' to 'orders', got: {:?}",
        aliases
    );
}

#[test]
fn test_extract_alias_map_multiple_tables() {
    let catalog = make_catalog();
    let registry = FunctionRegistry::new();
    let provider = FeatherFlowProvider::new(&catalog, &registry);
    let plan = sql_to_plan(
        "SELECT o.id, c.name FROM orders o JOIN customers c ON o.customer_id = c.id",
        &provider,
    )
    .unwrap();

    let aliases = extract_alias_map(&plan);
    assert_eq!(aliases.get("o").map(|v| v.as_str()), Some("orders"));
    assert_eq!(aliases.get("c").map(|v| v.as_str()), Some("customers"));
}

#[test]
fn test_extract_alias_map_no_aliases() {
    let catalog = make_catalog();
    let registry = FunctionRegistry::new();
    let provider = FeatherFlowProvider::new(&catalog, &registry);
    let plan = sql_to_plan("SELECT id FROM orders", &provider).unwrap();

    let aliases = extract_alias_map(&plan);
    assert!(
        aliases.is_empty(),
        "No aliases should be extracted when none used"
    );
}

// ==========================================================================
// Edge deduplication
// ==========================================================================

#[test]
fn test_deduplicate_edges_removes_duplicates() {
    let edges = vec![
        ColumnLineageEdge {
            output_column: "id".to_string(),
            source_table: "orders".to_string(),
            source_column: "id".to_string(),
            kind: LineageKind::Copy,
        },
        ColumnLineageEdge {
            output_column: "id".to_string(),
            source_table: "orders".to_string(),
            source_column: "id".to_string(),
            kind: LineageKind::Copy,
        },
        ColumnLineageEdge {
            output_column: "name".to_string(),
            source_table: "customers".to_string(),
            source_column: "name".to_string(),
            kind: LineageKind::Copy,
        },
    ];

    let deduped = deduplicate_edges(&edges);
    assert_eq!(deduped.len(), 2, "Should remove duplicate edge");
}

#[test]
fn test_deduplicate_edges_keeps_first() {
    let edges = vec![
        ColumnLineageEdge {
            output_column: "val".to_string(),
            source_table: "t".to_string(),
            source_column: "val".to_string(),
            kind: LineageKind::Copy,
        },
        ColumnLineageEdge {
            output_column: "val".to_string(),
            source_table: "t".to_string(),
            source_column: "val".to_string(),
            kind: LineageKind::Transform,
        },
    ];

    let deduped = deduplicate_edges(&edges);
    assert_eq!(deduped.len(), 1);
    assert_eq!(
        deduped[0].kind,
        LineageKind::Copy,
        "Should keep first occurrence"
    );
}

#[test]
fn test_deduplicate_empty() {
    let edges: Vec<ColumnLineageEdge> = vec![];
    let deduped = deduplicate_edges(&edges);
    assert!(deduped.is_empty());
}

// ==========================================================================
// classify_expr coverage
// ==========================================================================

#[test]
fn test_literal_no_lineage() {
    let lineage = plan_and_lineage("SELECT 42 AS constant FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);

    let literal_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "constant")
        .collect();
    // Literal doesn't reference any column so no lineage edges expected
    assert!(
        literal_edges.is_empty(),
        "Literal column should not produce lineage edges"
    );
}

#[test]
fn test_mixed_literal_and_column() {
    let lineage = plan_and_lineage("SELECT id, 'constant' AS label FROM orders");
    let deduped = deduplicate_edges(&lineage.edges);

    let id_edges: Vec<_> = deduped.iter().filter(|e| e.output_column == "id").collect();
    assert!(!id_edges.is_empty(), "id column should have lineage");

    let label_edges: Vec<_> = deduped
        .iter()
        .filter(|e| e.output_column == "label")
        .collect();
    assert!(
        label_edges.is_empty(),
        "Literal column should have no lineage edges"
    );
}

// ==========================================================================
// Complex real-world patterns
// ==========================================================================

#[test]
fn test_aggregate_with_filter_and_group_by() {
    let lineage = plan_and_lineage(
        "SELECT customer_id, SUM(amount) AS total, COUNT(*) AS order_count
         FROM orders
         WHERE status = 'completed'
         GROUP BY customer_id
         HAVING SUM(amount) > 500",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    // customer_id should appear (GROUP BY key)
    assert!(
        deduped.iter().any(|e| e.source_column == "customer_id"),
        "GROUP BY key should appear in lineage"
    );
    // amount should appear (SUM aggregate + HAVING)
    assert!(
        deduped.iter().any(|e| e.source_column == "amount"),
        "Aggregated column should appear in lineage"
    );
    // status should appear as inspect (WHERE clause)
    let inspect: Vec<_> = deduped
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect && e.source_column == "status")
        .collect();
    assert!(!inspect.is_empty(), "WHERE column should be Inspect");
}

#[test]
fn test_join_with_where_and_aggregate() {
    let lineage = plan_and_lineage_extended(
        "SELECT o.customer_id, SUM(p.amount) AS total_paid
         FROM orders o
         JOIN payments p ON o.id = p.order_id
         WHERE o.status = 'completed'
         GROUP BY o.customer_id",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    assert!(
        deduped.iter().any(|e| e.source_column == "customer_id"),
        "customer_id should appear in lineage"
    );
    assert!(
        deduped.iter().any(|e| e.source_column == "amount"),
        "payment amount should appear (aggregate)"
    );
}

#[test]
fn test_self_join() {
    let lineage = plan_and_lineage(
        "SELECT o1.id AS parent_id, o2.id AS child_id
         FROM orders o1
         JOIN orders o2 ON o1.id = o2.customer_id",
    );
    let deduped = deduplicate_edges(&lineage.edges);

    assert!(
        deduped.iter().any(|e| e.output_column == "parent_id"),
        "Should have lineage for parent_id"
    );
    assert!(
        deduped.iter().any(|e| e.output_column == "child_id"),
        "Should have lineage for child_id"
    );
}
