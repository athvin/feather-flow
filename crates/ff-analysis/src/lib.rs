//! ff-analysis: Static analysis for SQL models using DataFusion LogicalPlans
//!
//! This crate provides composable analysis passes that operate on DataFusion
//! LogicalPlans, type/schema definitions shared with the DataFusion bridge,
//! and schema propagation infrastructure.

pub(crate) mod context;
pub(crate) mod datafusion_bridge;
pub(crate) mod error;
pub(crate) mod pass;
pub mod schema;
pub mod types;

#[cfg(any(test, feature = "test-support"))]
pub mod test_utils;

pub use context::AnalysisContext;
pub use error::{AnalysisError, AnalysisResult};
pub use pass::plan_pass::{DagPlanPass, PlanPass, PlanPassManager};
pub use pass::{
    apply_severity_overrides, Diagnostic, DiagnosticCode, OverriddenSeverity, Severity,
    SeverityOverrides,
};
pub use schema::{RelSchema, SchemaCatalog};
pub use types::{parse_sql_type, FloatBitWidth, IntBitWidth, Nullability, SqlType, TypedColumn};

pub use datafusion_bridge::lineage::{
    deduplicate_edges, extract_alias_map, extract_column_lineage as extract_plan_column_lineage,
    ColumnLineageEdge, LineageKind, ModelColumnLineage,
};
pub use datafusion_bridge::planner::{plan_statement, sql_to_plan};
pub use datafusion_bridge::propagation::{
    propagate_schemas, ModelPlanResult, PropagationResult, SchemaMismatch,
};
pub use datafusion_bridge::provider::{
    build_user_function_stubs, FeatherFlowProvider, FunctionRegistry, UserFunctionStub,
    UserTableFunctionStub,
};
// Re-export the exact sqlparser DataFusion bundles (0.59), so a caller that
// rewrites the AST before `plan_statement` names the right `Statement` type and
// dialects without depending on datafusion-expr (or the wrong sqlparser) itself.
pub use datafusion_expr::sqlparser;
