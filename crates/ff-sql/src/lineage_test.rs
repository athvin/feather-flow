use super::*;
use crate::parser::SqlParser;

fn parse_and_extract_lineage(sql: &str, model_name: &str) -> Option<ModelLineage> {
    let parser = SqlParser::duckdb();
    let stmts = parser.parse(sql).unwrap();
    stmts
        .first()
        .and_then(|stmt| extract_column_lineage(stmt, model_name))
}

#[test]
fn test_simple_select() {
    let lineage = parse_and_extract_lineage("SELECT id, name FROM users", "test_model").unwrap();

    assert_eq!(lineage.model_name, "test_model");
    assert_eq!(lineage.columns.len(), 2);

    let id_col = lineage.get_column("id").unwrap();
    assert!(id_col.is_direct);
    assert_eq!(id_col.expr_type, ExprType::Column);

    let name_col = lineage.get_column("name").unwrap();
    assert!(name_col.is_direct);
}

#[test]
fn test_aliased_columns() {
    let lineage = parse_and_extract_lineage(
        "SELECT id AS user_id, name AS user_name FROM users",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 2);

    let user_id = lineage.get_column("user_id").unwrap();
    assert!(user_id.is_direct);
    assert!(user_id.source_columns.contains(&ColumnRef::simple("id")));

    let user_name = lineage.get_column("user_name").unwrap();
    assert!(user_name
        .source_columns
        .contains(&ColumnRef::simple("name")));
}

#[test]
fn test_qualified_column_refs() {
    let lineage =
        parse_and_extract_lineage("SELECT u.id, u.name FROM users u", "test_model").unwrap();

    assert_eq!(lineage.columns.len(), 2);

    // Check that aliases are resolved
    let id_col = lineage.get_column("id").unwrap();
    assert!(id_col
        .source_columns
        .contains(&ColumnRef::qualified("users", "id")));
}

#[test]
fn test_function_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT COUNT(id) AS cnt, UPPER(name) AS upper_name FROM users",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 2);

    let cnt = lineage.get_column("cnt").unwrap();
    assert!(!cnt.is_direct);
    assert_eq!(cnt.expr_type, ExprType::Function);
    assert!(cnt.source_columns.contains(&ColumnRef::simple("id")));

    let upper_name = lineage.get_column("upper_name").unwrap();
    assert!(!upper_name.is_direct);
    assert_eq!(upper_name.expr_type, ExprType::Function);
}

#[test]
fn test_expression_lineage() {
    let lineage =
        parse_and_extract_lineage("SELECT price * quantity AS total FROM orders", "test_model")
            .unwrap();

    let total = lineage.get_column("total").unwrap();
    assert!(!total.is_direct);
    assert_eq!(total.expr_type, ExprType::Expression);
    assert!(total.source_columns.contains(&ColumnRef::simple("price")));
    assert!(total
        .source_columns
        .contains(&ColumnRef::simple("quantity")));
}

#[test]
fn test_case_expression() {
    let lineage = parse_and_extract_lineage(
        "SELECT CASE WHEN status = 'active' THEN 1 ELSE 0 END AS is_active FROM users",
        "test_model",
    )
    .unwrap();

    let is_active = lineage.get_column("is_active").unwrap();
    assert!(!is_active.is_direct);
    assert_eq!(is_active.expr_type, ExprType::Case);
    assert!(is_active
        .source_columns
        .contains(&ColumnRef::simple("status")));
}

#[test]
fn test_join_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT o.id AS order_id, c.name AS customer_name
         FROM orders o
         JOIN customers c ON o.customer_id = c.id",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.source_tables.len(), 2);
    assert!(lineage.source_tables.contains("orders"));
    assert!(lineage.source_tables.contains("customers"));

    let order_id = lineage.get_column("order_id").unwrap();
    assert!(order_id
        .source_columns
        .contains(&ColumnRef::qualified("orders", "id")));

    let customer_name = lineage.get_column("customer_name").unwrap();
    assert!(customer_name
        .source_columns
        .contains(&ColumnRef::qualified("customers", "name")));
}

#[test]
fn test_wildcard_select() {
    let lineage = parse_and_extract_lineage("SELECT * FROM users", "test_model").unwrap();

    assert_eq!(lineage.columns.len(), 1);
    let wildcard = &lineage.columns[0];
    assert_eq!(wildcard.output_column, "*");
    assert_eq!(wildcard.expr_type, ExprType::Wildcard);
}

#[test]
fn test_literal_column() {
    let lineage = parse_and_extract_lineage(
        "SELECT id, 'constant' AS const_col, 42 AS num_col FROM users",
        "test_model",
    )
    .unwrap();

    let const_col = lineage.get_column("const_col").unwrap();
    assert!(!const_col.is_direct);
    assert_eq!(const_col.expr_type, ExprType::Literal);
    assert!(const_col.source_columns.is_empty());
}

#[test]
fn test_cast_expression() {
    let lineage = parse_and_extract_lineage(
        "SELECT CAST(amount AS DECIMAL(10,2)) AS amount_decimal FROM orders",
        "test_model",
    )
    .unwrap();

    let amount = lineage.get_column("amount_decimal").unwrap();
    assert_eq!(amount.expr_type, ExprType::Cast);
    assert!(amount.source_columns.contains(&ColumnRef::simple("amount")));
}

#[test]
fn test_cte_lineage() {
    let lineage = parse_and_extract_lineage(
        "WITH staged AS (SELECT id, name FROM raw_users)
         SELECT id AS user_id, name AS user_name FROM staged",
        "test_model",
    )
    .unwrap();

    // The main query references the CTE "staged" as its source
    // (raw_users is inside the CTE, so it's not directly visible in the main select's FROM)
    assert!(lineage.source_tables.contains("staged"));

    // Output columns should exist
    let user_id = lineage.get_column("user_id");
    assert!(user_id.is_some());
}

// --- Recursive traversal tests ---

/// Helper to build a ProjectLineage with given edges
fn build_project_lineage(edges: Vec<LineageEdge>) -> ProjectLineage {
    ProjectLineage {
        models: HashMap::new(),
        edges,
    }
}

fn make_edge(src_model: &str, src_col: &str, tgt_model: &str, tgt_col: &str) -> LineageEdge {
    let kind = if src_col == tgt_col {
        LineageKind::Copy
    } else {
        LineageKind::Rename
    };
    LineageEdge {
        source_model: src_model.to_string(),
        source_column: src_col.to_string(),
        target_model: tgt_model.to_string(),
        target_column: tgt_col.to_string(),
        is_direct: true,
        expr_type: ExprType::Column,
        kind,
        description_status: DescriptionStatus::Missing,
        classification: None,
    }
}

#[test]
fn test_recursive_downstream_three_model_chain() {
    // A.x → B.x → C.x
    let lineage = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        make_edge("B", "x", "C", "x"),
    ]);

    let downstream = lineage.column_consumers_recursive("A", "x");
    assert_eq!(downstream.len(), 2);
    assert!(downstream.iter().any(|e| e.target_model == "B"));
    assert!(downstream.iter().any(|e| e.target_model == "C"));
}

#[test]
fn test_recursive_upstream_three_model_chain() {
    // A.x → B.x → C.x
    let lineage = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        make_edge("B", "x", "C", "x"),
    ]);

    let upstream = lineage.trace_column_recursive("C", "x");
    assert_eq!(upstream.len(), 2);
    assert!(upstream.iter().any(|e| e.source_model == "A"));
    assert!(upstream.iter().any(|e| e.source_model == "B"));
}

#[test]
fn test_recursive_downstream_diamond() {
    // A.x → B.x, A.x → C.x, B.x → D.x, C.x → D.x
    let lineage = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        make_edge("A", "x", "C", "x"),
        make_edge("B", "x", "D", "x"),
        make_edge("C", "x", "D", "x"),
    ]);

    let downstream = lineage.column_consumers_recursive("A", "x");
    // Should get all 4 edges, no duplicates
    assert_eq!(downstream.len(), 4);
    // D.x appears as target twice (from B and C) — both edges should be present
    let d_edges: Vec<_> = downstream
        .iter()
        .filter(|e| e.target_model == "D")
        .collect();
    assert_eq!(d_edges.len(), 2);
}

#[test]
fn test_recursive_upstream_diamond() {
    // A.x → C.x, B.x → C.x, C.x → D.x
    let lineage = build_project_lineage(vec![
        make_edge("A", "x", "C", "x"),
        make_edge("B", "x", "C", "x"),
        make_edge("C", "x", "D", "x"),
    ]);

    let upstream = lineage.trace_column_recursive("D", "x");
    assert_eq!(upstream.len(), 3);
    assert!(upstream.iter().any(|e| e.source_model == "A"));
    assert!(upstream.iter().any(|e| e.source_model == "B"));
    assert!(upstream.iter().any(|e| e.source_model == "C"));
}

#[test]
fn test_recursive_cycle_protection() {
    // A.x → B.x → A.x (cycle)
    let lineage = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        make_edge("B", "x", "A", "x"),
    ]);

    // Should not loop forever; visited set prevents re-expansion
    let downstream = lineage.column_consumers_recursive("A", "x");
    assert_eq!(downstream.len(), 2);

    let upstream = lineage.trace_column_recursive("A", "x");
    assert_eq!(upstream.len(), 2);
}

#[test]
fn test_recursive_single_hop() {
    // Only one hop: A.x → B.x
    let lineage = build_project_lineage(vec![make_edge("A", "x", "B", "x")]);

    let downstream = lineage.column_consumers_recursive("A", "x");
    assert_eq!(downstream.len(), 1);
    assert_eq!(downstream[0].target_model, "B");

    let upstream = lineage.trace_column_recursive("B", "x");
    assert_eq!(upstream.len(), 1);
    assert_eq!(upstream[0].source_model, "A");
}

#[test]
fn test_recursive_no_matches() {
    let lineage = build_project_lineage(vec![make_edge("A", "x", "B", "x")]);

    // No downstream from B.x
    let downstream = lineage.column_consumers_recursive("B", "x");
    assert!(downstream.is_empty());

    // No upstream to A.x
    let upstream = lineage.trace_column_recursive("A", "x");
    assert!(upstream.is_empty());
}

// --- Unqualified column inference tests ---

#[test]
fn test_bare_column_resolves_to_single_source_table() {
    // stg_customers: SELECT id AS customer_id FROM raw_customers
    // `id` has no table qualifier, but source_tables has exactly one entry
    let lineage = parse_and_extract_lineage(
        "SELECT id AS customer_id FROM raw_customers",
        "stg_customers",
    )
    .unwrap();

    assert_eq!(lineage.source_tables.len(), 1);
    assert!(lineage.source_tables.contains("raw_customers"));

    let customer_id = lineage.get_column("customer_id").unwrap();
    // source_ref.table is None for bare `id`
    assert!(customer_id
        .source_columns
        .contains(&ColumnRef::simple("id")));

    // Now resolve edges — bare column should infer raw_customers
    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);

    let known: HashSet<&str> = ["raw_customers", "stg_customers"].iter().copied().collect();
    project.resolve_edges(&known);

    // Should create edge: raw_customers.id → stg_customers.customer_id
    assert_eq!(project.edges.len(), 1);
    assert_eq!(project.edges[0].source_model, "raw_customers");
    assert_eq!(project.edges[0].source_column, "id");
    assert_eq!(project.edges[0].target_model, "stg_customers");
    assert_eq!(project.edges[0].target_column, "customer_id");
}

#[test]
fn test_bare_column_with_multiple_sources_stays_unresolved() {
    // When there are 2+ source tables, bare column cannot be resolved
    let lineage =
        parse_and_extract_lineage("SELECT id, name FROM table_a, table_b", "test_model").unwrap();

    assert_eq!(lineage.source_tables.len(), 2);

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);

    let known: HashSet<&str> = ["table_a", "table_b"].iter().copied().collect();
    project.resolve_edges(&known);

    // No edges should be created — ambiguous source
    assert!(project.edges.is_empty());
}

#[test]
fn test_multi_hop_chain_with_bare_columns() {
    // Simulate: raw_customers -> stg_customers -> int_customer_ranking
    // stg_customers: SELECT id AS customer_id FROM raw_customers (bare column)
    // int_customer_ranking: SELECT c.customer_id FROM stg_customers c (qualified)

    let stg = parse_and_extract_lineage(
        "SELECT id AS customer_id FROM raw_customers",
        "stg_customers",
    )
    .unwrap();

    let int_rank = parse_and_extract_lineage(
        "SELECT c.customer_id FROM stg_customers c",
        "int_customer_ranking",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(stg);
    project.add_model_lineage(int_rank);

    let known: HashSet<&str> = ["raw_customers", "stg_customers", "int_customer_ranking"]
        .iter()
        .copied()
        .collect();
    project.resolve_edges(&known);

    // Should have 2 edges forming the full chain
    assert_eq!(project.edges.len(), 2);

    // Trace upstream from int_customer_ranking.customer_id should find both hops
    let chain = project.trace_column_recursive("int_customer_ranking", "customer_id");
    assert_eq!(chain.len(), 2, "Expected 2-hop chain, got {}", chain.len());

    // Verify the chain: raw_customers.id -> stg_customers.customer_id -> int_customer_ranking.customer_id
    assert!(chain
        .iter()
        .any(|e| e.source_model == "raw_customers" && e.source_column == "id"));
    assert!(chain
        .iter()
        .any(|e| e.source_model == "stg_customers" && e.source_column == "customer_id"));
}

// --- LineageKind tests ---

#[test]
fn test_lineage_kind_copy() {
    // SELECT customer_id FROM stg_customers — same name = Copy
    let lineage =
        parse_and_extract_lineage("SELECT customer_id FROM stg_customers", "test_model").unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["stg_customers"].iter().copied().collect();
    project.resolve_edges(&known);

    assert_eq!(project.edges.len(), 1);
    assert_eq!(project.edges[0].kind, LineageKind::Copy);
}

#[test]
fn test_lineage_kind_rename() {
    // SELECT id AS customer_id FROM raw_customers — different name = Rename
    let lineage = parse_and_extract_lineage(
        "SELECT id AS customer_id FROM raw_customers",
        "stg_customers",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["raw_customers"].iter().copied().collect();
    project.resolve_edges(&known);

    assert_eq!(project.edges.len(), 1);
    assert_eq!(project.edges[0].kind, LineageKind::Rename);
}

#[test]
fn test_lineage_kind_transform() {
    // SELECT COUNT(id) AS cnt FROM orders — function = Transform
    let lineage =
        parse_and_extract_lineage("SELECT COUNT(o.id) AS cnt FROM orders o", "test_model").unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["orders"].iter().copied().collect();
    project.resolve_edges(&known);

    assert_eq!(project.edges.len(), 1);
    assert_eq!(project.edges[0].kind, LineageKind::Transform);
}

// --- Inspect edge tests ---

#[test]
fn test_inspect_edges_from_where() {
    // WHERE references status which is NOT in SELECT output
    let lineage = parse_and_extract_lineage(
        "SELECT o.order_id FROM orders o WHERE o.status = 'completed'",
        "test_model",
    )
    .unwrap();

    // status should be in inspect_columns
    assert!(
        lineage.inspect_columns.iter().any(|r| r.column == "status"),
        "WHERE column 'status' should be an inspect column"
    );

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["orders"].iter().copied().collect();
    project.resolve_edges(&known);

    // Should have 1 Copy edge (order_id) + 1 Inspect edge (status)
    let copy_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.kind != LineageKind::Inspect)
        .collect();
    let inspect_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect)
        .collect();
    assert_eq!(copy_edges.len(), 1, "Expected 1 copy edge for order_id");
    assert_eq!(inspect_edges.len(), 1, "Expected 1 inspect edge for status");
    assert_eq!(inspect_edges[0].source_column, "status");
}

#[test]
fn test_inspect_edges_from_join_on() {
    // JOIN ON references customer_id in both tables, but only SELECT output from one
    let lineage = parse_and_extract_lineage(
        "SELECT c.customer_name FROM stg_customers c
         INNER JOIN stg_orders o ON c.customer_id = o.customer_id",
        "test_model",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["stg_customers", "stg_orders"].iter().copied().collect();
    project.resolve_edges(&known);

    // Should have inspect edges for customer_id from JOIN ON
    let inspect_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect)
        .collect();
    assert!(
        inspect_edges
            .iter()
            .any(|e| e.source_column == "customer_id"),
        "JOIN ON customer_id should create inspect edges"
    );
}

#[test]
fn test_inspect_edges_from_group_by() {
    // GROUP BY customer_id, but customer_id is also in SELECT — should NOT be inspect
    // HAVING uses total which is in SELECT — should NOT be inspect
    let lineage = parse_and_extract_lineage(
        "SELECT o.customer_id, count(o.order_id) as total
         FROM orders o
         GROUP BY o.customer_id
         HAVING count(o.order_id) > 5",
        "test_model",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["orders"].iter().copied().collect();
    project.resolve_edges(&known);

    // customer_id is in both SELECT and GROUP BY — should NOT produce an inspect edge
    let inspect_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect)
        .collect();
    assert!(
        !inspect_edges
            .iter()
            .any(|e| e.source_column == "customer_id"),
        "customer_id is already in SELECT, should not be an inspect edge"
    );
}

#[test]
fn test_no_inspect_for_columns_already_in_select() {
    // WHERE references order_id which IS in SELECT — should NOT be inspect
    let lineage = parse_and_extract_lineage(
        "SELECT o.order_id, o.amount FROM orders o WHERE o.order_id > 100",
        "test_model",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["orders"].iter().copied().collect();
    project.resolve_edges(&known);

    let inspect_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect)
        .collect();
    assert!(
        inspect_edges.is_empty(),
        "No inspect edges expected — all WHERE columns are already in SELECT"
    );
}

// ==========================================================================
// UNION / INTERSECT / EXCEPT
// ==========================================================================

#[test]
fn test_union_all_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT order_id, customer_id, 'completed' AS source FROM completed_orders
         UNION ALL
         SELECT order_id, customer_id, 'pending' AS source FROM pending_orders",
        "all_orders",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 3);
    let order_id = lineage.get_column("order_id").unwrap();
    assert!(order_id.is_direct);

    let source = lineage.get_column("source").unwrap();
    assert_eq!(source.expr_type, ExprType::Literal);
}

#[test]
fn test_union_lineage_uses_left_column_names() {
    let lineage = parse_and_extract_lineage(
        "SELECT id AS order_id FROM table_a
         UNION ALL
         SELECT oid AS order_id FROM table_b",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 1);
    let col = lineage.get_column("order_id").unwrap();
    assert!(col.source_columns.contains(&ColumnRef::simple("id")));
}

#[test]
fn test_triple_union() {
    let lineage = parse_and_extract_lineage(
        "SELECT id FROM a
         UNION ALL
         SELECT id FROM b
         UNION ALL
         SELECT id FROM c",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 1);
    assert!(lineage.source_tables.contains("a"));
}

#[test]
fn test_intersect_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT customer_id FROM vip_customers
         INTERSECT
         SELECT customer_id FROM active_customers",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 1);
    let col = lineage.get_column("customer_id").unwrap();
    assert!(col.is_direct);
}

#[test]
fn test_except_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT email FROM all_users
         EXCEPT
         SELECT email FROM unsubscribed",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 1);
    let col = lineage.get_column("email").unwrap();
    assert!(col.is_direct);
}

// ==========================================================================
// Subquery expressions
// ==========================================================================

#[test]
fn test_scalar_subquery_in_select() {
    let lineage = parse_and_extract_lineage(
        "SELECT id, (SELECT MAX(amount) FROM orders WHERE orders.customer_id = customers.id) AS max_order
         FROM customers",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 2);
    let max_order = lineage.get_column("max_order").unwrap();
    assert_eq!(max_order.expr_type, ExprType::Subquery);
    assert!(max_order
        .source_columns
        .iter()
        .any(|c| c.column == "amount"));
}

#[test]
fn test_derived_table_subquery() {
    let lineage = parse_and_extract_lineage(
        "SELECT sub.total_amount FROM (SELECT SUM(amount) AS total_amount FROM orders) sub",
        "test_model",
    )
    .unwrap();

    // The alias "sub" should be tracked
    assert!(lineage.table_aliases.contains_key("sub"));
    // The inner subquery's column and outer select's column both appear in lineage.
    // The outer SELECT's `sub.total_amount` resolves through the alias to the subquery.
    assert!(lineage
        .columns
        .iter()
        .any(|c| c.output_column == "total_amount"));
}

// ==========================================================================
// Multiple JOINs
// ==========================================================================

#[test]
fn test_three_table_join() {
    let lineage = parse_and_extract_lineage(
        "SELECT o.order_id, c.customer_name, p.product_name
         FROM orders o
         JOIN customers c ON o.customer_id = c.id
         JOIN products p ON o.product_id = p.id",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.source_tables.len(), 3);
    assert!(lineage.source_tables.contains("orders"));
    assert!(lineage.source_tables.contains("customers"));
    assert!(lineage.source_tables.contains("products"));

    let order_id = lineage.get_column("order_id").unwrap();
    assert!(order_id
        .source_columns
        .contains(&ColumnRef::qualified("orders", "order_id")));
    let customer_name = lineage.get_column("customer_name").unwrap();
    assert!(customer_name
        .source_columns
        .contains(&ColumnRef::qualified("customers", "customer_name")));
    let product_name = lineage.get_column("product_name").unwrap();
    assert!(product_name
        .source_columns
        .contains(&ColumnRef::qualified("products", "product_name")));
}

#[test]
fn test_left_join_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT o.id, c.name
         FROM orders o
         LEFT JOIN customers c ON o.customer_id = c.id",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.source_tables.len(), 2);
    let id_col = lineage.get_column("id").unwrap();
    assert!(id_col
        .source_columns
        .contains(&ColumnRef::qualified("orders", "id")));
}

#[test]
fn test_full_outer_join_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT a.id AS a_id, b.id AS b_id
         FROM table_a a
         FULL OUTER JOIN table_b b ON a.key = b.key",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.source_tables.len(), 2);
    assert_eq!(lineage.columns.len(), 2);
}

// ==========================================================================
// Nested CASE and complex expressions
// ==========================================================================

#[test]
fn test_nested_case_expression() {
    let lineage = parse_and_extract_lineage(
        "SELECT CASE
            WHEN category = 'electronics' THEN
                CASE WHEN price > 100 THEN 'premium' ELSE 'standard' END
            ELSE 'other'
         END AS detailed_category
         FROM products",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("detailed_category").unwrap();
    assert_eq!(col.expr_type, ExprType::Case);
    assert!(col.source_columns.contains(&ColumnRef::simple("category")));
    assert!(col.source_columns.contains(&ColumnRef::simple("price")));
}

#[test]
fn test_case_with_operand() {
    let lineage = parse_and_extract_lineage(
        "SELECT CASE status
            WHEN 'active' THEN 1
            WHEN 'inactive' THEN 0
            ELSE -1
         END AS status_code
         FROM users",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("status_code").unwrap();
    assert_eq!(col.expr_type, ExprType::Case);
    assert!(col.source_columns.contains(&ColumnRef::simple("status")));
}

// ==========================================================================
// Nested function calls
// ==========================================================================

#[test]
fn test_coalesce_wrapping_aggregate() {
    let lineage = parse_and_extract_lineage(
        "SELECT COALESCE(SUM(amount), 0) AS total_amount FROM orders",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("total_amount").unwrap();
    assert_eq!(col.expr_type, ExprType::Function);
    assert!(col.source_columns.iter().any(|c| c.column == "amount"));
}

#[test]
fn test_multi_arg_function() {
    let lineage = parse_and_extract_lineage(
        "SELECT CONCAT(first_name, ' ', last_name) AS full_name FROM users",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("full_name").unwrap();
    assert_eq!(col.expr_type, ExprType::Function);
    assert!(col
        .source_columns
        .contains(&ColumnRef::simple("first_name")));
    assert!(col.source_columns.contains(&ColumnRef::simple("last_name")));
}

#[test]
fn test_count_star() {
    let lineage =
        parse_and_extract_lineage("SELECT COUNT(*) AS total FROM orders", "test_model").unwrap();

    let col = lineage.get_column("total").unwrap();
    assert_eq!(col.expr_type, ExprType::Function);
    assert!(col.source_columns.contains(&ColumnRef::simple("*")));
}

// ==========================================================================
// IS NULL / IS NOT NULL
// ==========================================================================

#[test]
fn test_is_null_expression() {
    let lineage =
        parse_and_extract_lineage("SELECT id FROM orders WHERE amount IS NULL", "test_model")
            .unwrap();

    assert!(
        lineage.inspect_columns.iter().any(|r| r.column == "amount"),
        "IS NULL column should appear as inspect"
    );
}

#[test]
fn test_is_not_null_expression() {
    let lineage = parse_and_extract_lineage(
        "SELECT id FROM orders WHERE email IS NOT NULL",
        "test_model",
    )
    .unwrap();

    assert!(
        lineage.inspect_columns.iter().any(|r| r.column == "email"),
        "IS NOT NULL column should appear as inspect"
    );
}

// ==========================================================================
// BETWEEN and IN LIST
// ==========================================================================

#[test]
fn test_between_in_where_creates_inspect() {
    let lineage = parse_and_extract_lineage(
        "SELECT o.order_id FROM orders o WHERE o.amount BETWEEN 100 AND 500",
        "test_model",
    )
    .unwrap();

    assert!(
        lineage.inspect_columns.iter().any(|r| r.column == "amount"),
        "BETWEEN column should appear as inspect"
    );
}

#[test]
fn test_between_with_column_bounds() {
    let lineage = parse_and_extract_lineage(
        "SELECT id FROM orders WHERE amount BETWEEN min_amount AND max_amount",
        "test_model",
    )
    .unwrap();

    let all_inspect_cols: Vec<&str> = lineage
        .inspect_columns
        .iter()
        .map(|r| r.column.as_str())
        .collect();
    assert!(
        all_inspect_cols.contains(&"amount"),
        "amount should be inspect"
    );
    assert!(
        all_inspect_cols.contains(&"min_amount"),
        "min_amount should be inspect"
    );
    assert!(
        all_inspect_cols.contains(&"max_amount"),
        "max_amount should be inspect"
    );
}

#[test]
fn test_in_list_in_where_creates_inspect() {
    let lineage = parse_and_extract_lineage(
        "SELECT o.id FROM orders o WHERE o.status IN ('completed', 'shipped', 'processing')",
        "test_model",
    )
    .unwrap();

    assert!(
        lineage.inspect_columns.iter().any(|r| r.column == "status"),
        "IN list column should appear as inspect"
    );
}

// ==========================================================================
// Qualified wildcard (table.*)
// ==========================================================================

#[test]
fn test_qualified_wildcard() {
    let lineage = parse_and_extract_lineage(
        "SELECT o.* FROM orders o JOIN customers c ON o.customer_id = c.id",
        "test_model",
    )
    .unwrap();

    // qualified wildcard produces one column entry
    let wildcard_cols: Vec<_> = lineage
        .columns
        .iter()
        .filter(|c| c.output_column.contains("*"))
        .collect();
    assert_eq!(wildcard_cols.len(), 1);
    let col = wildcard_cols[0];
    assert_eq!(col.expr_type, ExprType::Wildcard);
    // The qualified wildcard uses the alias name "o" as-is (not resolved to "orders")
    assert!(col.source_columns.contains(&ColumnRef::qualified("o", "*")));
}

// ==========================================================================
// Multiple CTEs
// ==========================================================================

#[test]
fn test_multiple_ctes() {
    let lineage = parse_and_extract_lineage(
        "WITH
         cte_orders AS (SELECT order_id, customer_id FROM raw_orders),
         cte_customers AS (SELECT id, name FROM raw_customers)
         SELECT o.order_id, c.name
         FROM cte_orders o
         JOIN cte_customers c ON o.customer_id = c.id",
        "test_model",
    )
    .unwrap();

    assert!(lineage.source_tables.contains("cte_orders"));
    assert!(lineage.source_tables.contains("cte_customers"));
    assert_eq!(lineage.columns.len(), 2);
}

#[test]
fn test_chained_ctes() {
    let lineage = parse_and_extract_lineage(
        "WITH
         base AS (SELECT id, amount FROM raw_orders),
         enriched AS (SELECT id, amount * 1.1 AS taxed_amount FROM base)
         SELECT id, taxed_amount FROM enriched",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("taxed_amount").unwrap();
    assert!(col.is_direct);
    assert!(lineage.source_tables.contains("enriched"));
}

// ==========================================================================
// Description status computation
// ==========================================================================

#[test]
fn test_compute_description_status_inherited() {
    let mut project = build_project_lineage(vec![make_edge("src", "col_a", "tgt", "col_a")]);
    let mut descriptions: HashMap<String, HashMap<String, String>> = HashMap::new();
    descriptions.insert(
        "src".to_string(),
        [("col_a".to_string(), "The A column".to_string())].into(),
    );
    descriptions.insert(
        "tgt".to_string(),
        [("col_a".to_string(), "The A column".to_string())].into(),
    );

    project.compute_description_status(&descriptions);
    assert_eq!(
        project.edges[0].description_status,
        DescriptionStatus::Inherited
    );
}

#[test]
fn test_compute_description_status_modified() {
    let mut project = build_project_lineage(vec![make_edge("src", "col_a", "tgt", "col_a")]);
    let mut descriptions: HashMap<String, HashMap<String, String>> = HashMap::new();
    descriptions.insert(
        "src".to_string(),
        [("col_a".to_string(), "Source description".to_string())].into(),
    );
    descriptions.insert(
        "tgt".to_string(),
        [("col_a".to_string(), "Different description".to_string())].into(),
    );

    project.compute_description_status(&descriptions);
    assert_eq!(
        project.edges[0].description_status,
        DescriptionStatus::Modified
    );
}

#[test]
fn test_compute_description_status_missing_source() {
    let mut project = build_project_lineage(vec![make_edge("src", "col_a", "tgt", "col_a")]);
    let mut descriptions: HashMap<String, HashMap<String, String>> = HashMap::new();
    descriptions.insert(
        "tgt".to_string(),
        [("col_a".to_string(), "Only target has desc".to_string())].into(),
    );

    project.compute_description_status(&descriptions);
    assert_eq!(
        project.edges[0].description_status,
        DescriptionStatus::Missing
    );
}

#[test]
fn test_compute_description_status_missing_target() {
    let mut project = build_project_lineage(vec![make_edge("src", "col_a", "tgt", "col_a")]);
    let mut descriptions: HashMap<String, HashMap<String, String>> = HashMap::new();
    descriptions.insert(
        "src".to_string(),
        [("col_a".to_string(), "Only source has desc".to_string())].into(),
    );

    project.compute_description_status(&descriptions);
    assert_eq!(
        project.edges[0].description_status,
        DescriptionStatus::Missing
    );
}

#[test]
fn test_compute_description_status_no_descriptions() {
    let mut project = build_project_lineage(vec![make_edge("src", "col_a", "tgt", "col_a")]);
    let descriptions: HashMap<String, HashMap<String, String>> = HashMap::new();

    project.compute_description_status(&descriptions);
    assert_eq!(
        project.edges[0].description_status,
        DescriptionStatus::Missing
    );
}

// ==========================================================================
// Classification propagation
// ==========================================================================

#[test]
fn test_propagate_classifications() {
    let mut project = build_project_lineage(vec![
        make_edge("src", "email", "tgt", "email"),
        make_edge("src", "id", "tgt", "id"),
    ]);

    let mut classifications: HashMap<String, HashMap<String, String>> = HashMap::new();
    classifications.insert(
        "src".to_string(),
        [("email".to_string(), "PII".to_string())].into(),
    );

    project.propagate_classifications(&classifications);

    let email_edge = project
        .edges
        .iter()
        .find(|e| e.source_column == "email")
        .unwrap();
    assert_eq!(email_edge.classification.as_deref(), Some("PII"));

    let id_edge = project
        .edges
        .iter()
        .find(|e| e.source_column == "id")
        .unwrap();
    assert!(id_edge.classification.is_none());
}

#[test]
fn test_classification_propagation_through_chain() {
    let mut project = build_project_lineage(vec![
        make_edge("raw", "ssn", "stg", "ssn"),
        make_edge("stg", "ssn", "fct", "ssn"),
    ]);

    let mut classifications: HashMap<String, HashMap<String, String>> = HashMap::new();
    classifications.insert(
        "raw".to_string(),
        [("ssn".to_string(), "SENSITIVE".to_string())].into(),
    );
    classifications.insert(
        "stg".to_string(),
        [("ssn".to_string(), "SENSITIVE".to_string())].into(),
    );

    project.propagate_classifications(&classifications);

    for edge in &project.edges {
        assert_eq!(
            edge.classification.as_deref(),
            Some("SENSITIVE"),
            "Classification should propagate: {} -> {}",
            edge.source_model,
            edge.target_model
        );
    }
}

// ==========================================================================
// DOT graph output
// ==========================================================================

#[test]
fn test_dot_output_basic() {
    let mut lineage_a = ModelLineage::new("model_a");
    lineage_a.add_column(ColumnLineage::direct("id", ColumnRef::simple("id")));

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage_a);
    project.edges.push(LineageEdge {
        source_model: "model_a".to_string(),
        source_column: "id".to_string(),
        target_model: "model_b".to_string(),
        target_column: "id".to_string(),
        is_direct: true,
        expr_type: ExprType::Column,
        kind: LineageKind::Copy,
        description_status: DescriptionStatus::Missing,
        classification: None,
    });

    let dot = project.to_dot();
    assert!(dot.contains("digraph lineage"));
    assert!(dot.contains("rankdir=LR"));
    assert!(dot.contains("model_a"));
    assert!(dot.contains("model_b"));
    assert!(dot.contains("copy"));
}

#[test]
fn test_dot_output_edge_styles() {
    let mut project = ProjectLineage::new();
    project.edges.push(LineageEdge {
        source_model: "a".to_string(),
        source_column: "x".to_string(),
        target_model: "b".to_string(),
        target_column: "y".to_string(),
        is_direct: true,
        expr_type: ExprType::Column,
        kind: LineageKind::Rename,
        description_status: DescriptionStatus::Missing,
        classification: None,
    });
    project.edges.push(LineageEdge {
        source_model: "a".to_string(),
        source_column: "x".to_string(),
        target_model: "c".to_string(),
        target_column: "z".to_string(),
        is_direct: false,
        expr_type: ExprType::Function,
        kind: LineageKind::Transform,
        description_status: DescriptionStatus::Missing,
        classification: None,
    });
    project.edges.push(LineageEdge {
        source_model: "a".to_string(),
        source_column: "x".to_string(),
        target_model: "d".to_string(),
        target_column: "x".to_string(),
        is_direct: false,
        expr_type: ExprType::Expression,
        kind: LineageKind::Inspect,
        description_status: DescriptionStatus::Missing,
        classification: None,
    });

    let dot = project.to_dot();
    assert!(dot.contains("rename"), "Should contain rename label");
    assert!(dot.contains("style=bold"), "Transform should be bold");
    assert!(dot.contains("style=dashed"), "Inspect should be dashed");
}

// ==========================================================================
// Case-insensitive model matching
// ==========================================================================

#[test]
fn test_case_insensitive_model_resolution() {
    let lineage =
        parse_and_extract_lineage("SELECT id FROM RAW_CUSTOMERS", "stg_customers").unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["raw_customers"].iter().copied().collect();
    project.resolve_edges(&known);

    assert_eq!(project.edges.len(), 1);
    assert_eq!(project.edges[0].source_model, "raw_customers");
}

// ==========================================================================
// Edge resolution with aliases
// ==========================================================================

#[test]
fn test_alias_resolution_in_edge_creation() {
    let lineage =
        parse_and_extract_lineage("SELECT o.id, o.amount FROM orders o", "stg_orders").unwrap();

    assert!(lineage.table_aliases.contains_key("o"));
    assert_eq!(lineage.table_aliases["o"], "orders");

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["orders"].iter().copied().collect();
    project.resolve_edges(&known);

    assert_eq!(project.edges.len(), 2);
    assert!(project.edges.iter().all(|e| e.source_model == "orders"));
}

// ==========================================================================
// Complex expression patterns
// ==========================================================================

#[test]
fn test_arithmetic_with_function_and_case() {
    let lineage = parse_and_extract_lineage(
        "SELECT
            CASE WHEN status = 'active' THEN amount * 1.1 ELSE 0 END
            + COALESCE(discount, 0) AS adjusted_total
         FROM orders",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("adjusted_total").unwrap();
    assert_eq!(col.expr_type, ExprType::Expression);
    assert!(col.source_columns.contains(&ColumnRef::simple("status")));
    assert!(col.source_columns.contains(&ColumnRef::simple("amount")));
    assert!(col.source_columns.contains(&ColumnRef::simple("discount")));
}

#[test]
fn test_unary_negation() {
    let lineage = parse_and_extract_lineage(
        "SELECT -amount AS negative_amount FROM orders",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("negative_amount").unwrap();
    assert_eq!(col.expr_type, ExprType::Expression);
    assert!(col.source_columns.contains(&ColumnRef::simple("amount")));
}

#[test]
fn test_nested_parentheses() {
    let lineage = parse_and_extract_lineage(
        "SELECT ((price + tax) * quantity) AS total FROM line_items",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("total").unwrap();
    assert_eq!(col.expr_type, ExprType::Expression);
    assert!(col.source_columns.contains(&ColumnRef::simple("price")));
    assert!(col.source_columns.contains(&ColumnRef::simple("tax")));
    assert!(col.source_columns.contains(&ColumnRef::simple("quantity")));
}

// ==========================================================================
// DISTINCT
// ==========================================================================

#[test]
fn test_select_distinct_lineage() {
    let lineage = parse_and_extract_lineage(
        "SELECT DISTINCT category, status FROM products",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 2);
    let cat = lineage.get_column("category").unwrap();
    assert!(cat.is_direct);
    let status = lineage.get_column("status").unwrap();
    assert!(status.is_direct);
}

// ==========================================================================
// Non-SELECT statements
// ==========================================================================

#[test]
fn test_non_query_returns_none() {
    let parser = SqlParser::duckdb();
    let stmts = parser.parse("CREATE TABLE foo (id INT)").unwrap();
    let result = extract_column_lineage(&stmts[0], "foo");
    assert!(
        result.is_none(),
        "CREATE TABLE should return None for lineage"
    );
}

#[test]
fn test_insert_returns_none() {
    let parser = SqlParser::duckdb();
    let stmts = parser.parse("INSERT INTO foo VALUES (1, 'a')").unwrap();
    let result = extract_column_lineage(&stmts[0], "foo");
    assert!(result.is_none(), "INSERT should return None for lineage");
}

// ==========================================================================
// Edge resolution with unknown tables (not in known_models)
// ==========================================================================

#[test]
fn test_unknown_source_table_produces_no_edge() {
    let lineage =
        parse_and_extract_lineage("SELECT e.id FROM external_api_table e", "test_model").unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["test_model"].iter().copied().collect();
    project.resolve_edges(&known);

    assert!(
        project.edges.is_empty(),
        "External table not in known_models should produce no edge"
    );
}

// ==========================================================================
// ProjectLineage trace_column (non-recursive)
// ==========================================================================

#[test]
fn test_trace_column_direct() {
    let project = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        make_edge("C", "y", "B", "y"),
        make_edge("B", "x", "D", "x"),
    ]);

    let upstream = project.trace_column("B", "x");
    assert_eq!(upstream.len(), 1);
    assert_eq!(upstream[0].source_model, "A");
}

#[test]
fn test_column_consumers_direct() {
    let project = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        make_edge("A", "x", "C", "x"),
        make_edge("A", "y", "D", "y"),
    ]);

    let downstream = project.column_consumers("A", "x");
    assert_eq!(downstream.len(), 2);
    assert!(downstream.iter().any(|e| e.target_model == "B"));
    assert!(downstream.iter().any(|e| e.target_model == "C"));
}

// ==========================================================================
// Inspect edges in complex scenarios
// ==========================================================================

#[test]
fn test_inspect_edges_from_having_with_different_column() {
    let lineage = parse_and_extract_lineage(
        "SELECT customer_id, SUM(amount) AS total
         FROM orders
         GROUP BY customer_id
         HAVING COUNT(order_id) > 3",
        "test_model",
    )
    .unwrap();

    assert!(
        lineage
            .inspect_columns
            .iter()
            .any(|r| r.column == "order_id"),
        "HAVING column 'order_id' not in SELECT should be inspect"
    );
}

#[test]
fn test_inspect_edges_from_multiple_joins() {
    let lineage = parse_and_extract_lineage(
        "SELECT c.name
         FROM customers c
         INNER JOIN orders o ON c.id = o.customer_id
         INNER JOIN payments p ON o.id = p.order_id",
        "test_model",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = ["customers", "orders", "payments"]
        .iter()
        .copied()
        .collect();
    project.resolve_edges(&known);

    let inspect_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.kind == LineageKind::Inspect)
        .collect();
    assert!(
        inspect_edges.len() >= 2,
        "Should have inspect edges from both JOIN ON clauses, got {}",
        inspect_edges.len()
    );
}

// ==========================================================================
// Realistic multi-model project lineage end-to-end
// ==========================================================================

#[test]
fn test_realistic_three_layer_project() {
    // raw_orders -> stg_orders -> fct_orders
    let stg = parse_and_extract_lineage(
        "SELECT id AS order_id, user_id AS customer_id, amount, status FROM raw_orders",
        "stg_orders",
    )
    .unwrap();

    let fct = parse_and_extract_lineage(
        "SELECT o.order_id, o.customer_id, o.amount, o.status,
                CASE WHEN o.status = 'completed' THEN 'done' ELSE 'pending' END AS status_label
         FROM stg_orders o
         WHERE o.amount > 0",
        "fct_orders",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(stg);
    project.add_model_lineage(fct);

    let known: HashSet<&str> = ["raw_orders", "stg_orders", "fct_orders"]
        .iter()
        .copied()
        .collect();
    project.resolve_edges(&known);

    // stg_orders: 4 rename edges from raw_orders
    let stg_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.target_model == "stg_orders" && e.kind != LineageKind::Inspect)
        .collect();
    assert_eq!(
        stg_edges.len(),
        4,
        "stg_orders should have 4 column edges from raw_orders"
    );

    // fct_orders: 4 copy edges + 1 transform (status_label) + inspect (amount from WHERE, status from CASE condition)
    let fct_copy_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.target_model == "fct_orders" && e.kind == LineageKind::Copy)
        .collect();
    assert_eq!(
        fct_copy_edges.len(),
        4,
        "fct_orders should have 4 copy edges"
    );

    let fct_transform_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.target_model == "fct_orders" && e.kind == LineageKind::Transform)
        .collect();
    assert!(
        !fct_transform_edges.is_empty(),
        "fct_orders should have transform edge for status_label"
    );

    // Recursive trace: trace fct_orders.order_id upstream to raw_orders
    let chain = project.trace_column_recursive("fct_orders", "order_id");
    assert_eq!(
        chain.len(),
        2,
        "order_id should trace 2 hops: stg_orders -> raw_orders"
    );
    assert!(chain
        .iter()
        .any(|e| e.source_model == "raw_orders" && e.source_column == "id"));
    assert!(chain
        .iter()
        .any(|e| e.source_model == "stg_orders" && e.source_column == "order_id"));
}

#[test]
fn test_realistic_join_model_project() {
    let stg_customers = parse_and_extract_lineage(
        "SELECT id AS customer_id, name AS customer_name FROM raw_customers",
        "stg_customers",
    )
    .unwrap();

    let stg_orders = parse_and_extract_lineage(
        "SELECT id AS order_id, user_id AS customer_id, amount FROM raw_orders",
        "stg_orders",
    )
    .unwrap();

    let fct = parse_and_extract_lineage(
        "SELECT o.order_id, o.customer_id, c.customer_name, o.amount
         FROM stg_orders o
         INNER JOIN stg_customers c ON o.customer_id = c.customer_id",
        "fct_customer_orders",
    )
    .unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(stg_customers);
    project.add_model_lineage(stg_orders);
    project.add_model_lineage(fct);

    let known: HashSet<&str> = [
        "raw_customers",
        "raw_orders",
        "stg_customers",
        "stg_orders",
        "fct_customer_orders",
    ]
    .iter()
    .copied()
    .collect();
    project.resolve_edges(&known);

    // fct_customer_orders should have edges from both stg models
    let fct_edges: Vec<_> = project
        .edges
        .iter()
        .filter(|e| e.target_model == "fct_customer_orders" && e.kind != LineageKind::Inspect)
        .collect();
    assert!(
        fct_edges.len() >= 4,
        "fct should have edges from both stg tables"
    );

    let from_stg_customers: Vec<_> = fct_edges
        .iter()
        .filter(|e| e.source_model == "stg_customers")
        .collect();
    assert!(
        !from_stg_customers.is_empty(),
        "Should have edges from stg_customers"
    );

    let from_stg_orders: Vec<_> = fct_edges
        .iter()
        .filter(|e| e.source_model == "stg_orders")
        .collect();
    assert!(
        !from_stg_orders.is_empty(),
        "Should have edges from stg_orders"
    );

    // Trace customer_name from fct all the way to raw_customers
    let chain = project.trace_column_recursive("fct_customer_orders", "customer_name");
    assert!(
        chain.len() >= 2,
        "customer_name should trace through stg_customers to raw_customers"
    );
    assert!(
        chain.iter().any(|e| e.source_model == "raw_customers"),
        "customer_name trace should reach raw_customers"
    );
}

// ==========================================================================
// Edge cases and boundary conditions
// ==========================================================================

#[test]
fn test_empty_project_lineage() {
    let project = ProjectLineage::new();
    assert!(project.edges.is_empty());
    assert!(project.models.is_empty());
    assert!(project.trace_column("anything", "col").is_empty());
    assert!(project.column_consumers("anything", "col").is_empty());
    assert!(project.trace_column_recursive("anything", "col").is_empty());
    assert!(project
        .column_consumers_recursive("anything", "col")
        .is_empty());
}

#[test]
fn test_model_with_only_literals() {
    let lineage = parse_and_extract_lineage(
        "SELECT 1 AS one, 'hello' AS greeting, 3.14 AS pi",
        "constants_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 3);
    for col in &lineage.columns {
        assert_eq!(col.expr_type, ExprType::Literal);
        assert!(col.source_columns.is_empty());
    }

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = HashSet::new();
    project.resolve_edges(&known);
    assert!(project.edges.is_empty());
}

#[test]
fn test_resolve_edges_with_no_known_models() {
    let lineage =
        parse_and_extract_lineage("SELECT id, name FROM some_table", "test_model").unwrap();

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);
    let known: HashSet<&str> = HashSet::new();
    project.resolve_edges(&known);

    assert!(project.edges.is_empty());
}

// ==========================================================================
// LineageKind Display and serialization
// ==========================================================================

#[test]
fn test_lineage_kind_display() {
    assert_eq!(format!("{}", LineageKind::Copy), "copy");
    assert_eq!(format!("{}", LineageKind::Rename), "rename");
    assert_eq!(format!("{}", LineageKind::Transform), "transform");
    assert_eq!(format!("{}", LineageKind::Inspect), "inspect");
}

#[test]
fn test_expr_type_display() {
    assert_eq!(format!("{}", ExprType::Column), "column");
    assert_eq!(format!("{}", ExprType::Function), "function");
    assert_eq!(format!("{}", ExprType::Literal), "literal");
    assert_eq!(format!("{}", ExprType::Expression), "expression");
    assert_eq!(format!("{}", ExprType::Wildcard), "wildcard");
    assert_eq!(format!("{}", ExprType::Cast), "cast");
    assert_eq!(format!("{}", ExprType::Case), "case");
    assert_eq!(format!("{}", ExprType::Subquery), "subquery");
    assert_eq!(format!("{}", ExprType::Unknown), "unknown");
}

#[test]
fn test_description_status_display() {
    assert_eq!(format!("{}", DescriptionStatus::Inherited), "inherited");
    assert_eq!(format!("{}", DescriptionStatus::Modified), "modified");
    assert_eq!(format!("{}", DescriptionStatus::Missing), "missing");
}

#[test]
fn test_column_ref_display() {
    let simple = ColumnRef::simple("id");
    assert_eq!(format!("{}", simple), "id");

    let qualified = ColumnRef::qualified("orders", "id");
    assert_eq!(format!("{}", qualified), "orders.id");
}

// ==========================================================================
// Recursive traversal with renamed columns
// ==========================================================================

#[test]
fn test_recursive_trace_through_rename() {
    // raw.id -> stg.customer_id -> fct.customer_id
    let project = build_project_lineage(vec![
        LineageEdge {
            source_model: "raw".to_string(),
            source_column: "id".to_string(),
            target_model: "stg".to_string(),
            target_column: "customer_id".to_string(),
            is_direct: true,
            expr_type: ExprType::Column,
            kind: LineageKind::Rename,
            description_status: DescriptionStatus::Missing,
            classification: None,
        },
        make_edge("stg", "customer_id", "fct", "customer_id"),
    ]);

    let upstream = project.trace_column_recursive("fct", "customer_id");
    assert_eq!(upstream.len(), 2);
    assert!(upstream
        .iter()
        .any(|e| e.source_model == "raw" && e.source_column == "id"));
}

#[test]
fn test_recursive_downstream_with_rename() {
    let project = build_project_lineage(vec![
        make_edge("A", "x", "B", "x"),
        LineageEdge {
            source_model: "B".to_string(),
            source_column: "x".to_string(),
            target_model: "C".to_string(),
            target_column: "y".to_string(),
            is_direct: true,
            expr_type: ExprType::Column,
            kind: LineageKind::Rename,
            description_status: DescriptionStatus::Missing,
            classification: None,
        },
    ]);

    let downstream = project.column_consumers_recursive("A", "x");
    assert_eq!(downstream.len(), 2);
    assert!(downstream
        .iter()
        .any(|e| e.target_model == "C" && e.target_column == "y"));
}

// ==========================================================================
// Multiple columns from same source (fan-out / fan-in)
// ==========================================================================

#[test]
fn test_fan_out_same_source_column() {
    // Same source column used in multiple output columns
    let lineage = parse_and_extract_lineage(
        "SELECT amount, amount * 0.1 AS tax, amount * 1.1 AS total_with_tax FROM orders",
        "test_model",
    )
    .unwrap();

    assert_eq!(lineage.columns.len(), 3);
    let amount = lineage.get_column("amount").unwrap();
    assert!(amount.is_direct);
    let tax = lineage.get_column("tax").unwrap();
    assert_eq!(tax.expr_type, ExprType::Expression);
    assert!(tax.source_columns.contains(&ColumnRef::simple("amount")));
    let total = lineage.get_column("total_with_tax").unwrap();
    assert_eq!(total.expr_type, ExprType::Expression);
    assert!(total.source_columns.contains(&ColumnRef::simple("amount")));
}

#[test]
fn test_fan_in_multiple_sources_to_one_output() {
    let lineage = parse_and_extract_lineage(
        "SELECT first_name || ' ' || last_name || ' <' || email || '>' AS contact_info FROM users",
        "test_model",
    )
    .unwrap();

    let col = lineage.get_column("contact_info").unwrap();
    assert_eq!(col.expr_type, ExprType::Expression);
    assert!(col
        .source_columns
        .contains(&ColumnRef::simple("first_name")));
    assert!(col.source_columns.contains(&ColumnRef::simple("last_name")));
    assert!(col.source_columns.contains(&ColumnRef::simple("email")));
}

// ==========================================================================
// DOT escape correctness
// ==========================================================================

#[test]
fn test_dot_escape_special_characters() {
    let mut lineage = ModelLineage::new("model<with>special");
    lineage.add_column(ColumnLineage::direct(
        "col|pipe",
        ColumnRef::simple("col|pipe"),
    ));

    let mut project = ProjectLineage::new();
    project.add_model_lineage(lineage);

    let dot = project.to_dot();
    assert!(
        dot.contains("model\\<with\\>special"),
        "Should escape < and >"
    );
    assert!(dot.contains("col\\|pipe"), "Should escape |");
}
