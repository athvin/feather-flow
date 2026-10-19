//! DuckDB function stubs for DataFusion planning
//!
//! Registers stub UDFs with correct signatures so that DataFusion's SqlToRel
//! can plan queries containing DuckDB-specific functions. These stubs are
//! never executed — they only provide type information for static analysis.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, FieldRef};
use datafusion_common::{DataFusionError, Result as DFResult};
use datafusion_expr::function::AccumulatorArgs;
use datafusion_expr::{
    Accumulator, AggregateUDF, ColumnarValue, PartitionEvaluator, ScalarUDF, ScalarUDFImpl,
    Signature, TypeSignature, Volatility, WindowUDF, WindowUDFImpl,
};
use datafusion_functions_window_common::field::WindowUDFFieldArgs;
use datafusion_functions_window_common::partition::PartitionEvaluatorArgs;

/// Register all DuckDB scalar UDFs
pub(crate) fn duckdb_scalar_udfs() -> Vec<Arc<ScalarUDF>> {
    let ts = DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None);
    let date = DataType::Date32;

    vec![
        make_overloaded_scalar(
            "date_trunc",
            &[
                (vec![DataType::Utf8, ts.clone()], ts.clone()),
                (vec![DataType::Utf8, date.clone()], date.clone()),
            ],
        ),
        make_overloaded_scalar(
            "date_part",
            &[
                (vec![DataType::Utf8, ts.clone()], DataType::Int64),
                (vec![DataType::Utf8, date.clone()], DataType::Int64),
            ],
        ),
        make_overloaded_scalar(
            "date_diff",
            &[
                (
                    vec![DataType::Utf8, ts.clone(), ts.clone()],
                    DataType::Int64,
                ),
                (
                    vec![DataType::Utf8, date.clone(), date.clone()],
                    DataType::Int64,
                ),
            ],
        ),
        make_scalar(
            "date_add",
            vec![
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
                DataType::Interval(arrow::datatypes::IntervalUnit::DayTime),
            ],
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        make_scalar(
            "datediff",
            vec![
                DataType::Utf8,
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            ],
            DataType::Int64,
        ),
        make_scalar(
            "dateadd",
            vec![
                DataType::Utf8,
                DataType::Int64,
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            ],
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        make_scalar(
            "strftime",
            vec![
                DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
                DataType::Utf8,
            ],
            DataType::Utf8,
        ),
        make_scalar(
            "strptime",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        make_scalar(
            "epoch",
            vec![DataType::Timestamp(
                arrow::datatypes::TimeUnit::Microsecond,
                None,
            )],
            DataType::Int64,
        ),
        make_scalar(
            "epoch_ms",
            vec![DataType::Int64],
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        make_scalar(
            "regexp_matches",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Boolean,
        ),
        // Variadic: DuckDB's regexp_replace takes an optional flags argument
        // (the standardize macros pass 'g').
        make_variadic_scalar("regexp_replace", DataType::Utf8),
        // Variadic: DuckDB's regexp_extract takes an optional group index.
        make_variadic_scalar("regexp_extract", DataType::Utf8),
        make_type_preserving_variadic_scalar("coalesce"),
        make_type_preserving_variadic_scalar("ifnull"),
        make_type_preserving_variadic_scalar("nullif"),
        make_variadic_scalar("struct_pack", DataType::Utf8),
        // JSON / list constructors + accessors the entity-resolution engine's
        // stage SQL uses (golden-mart assembly): all polymorphic, so variadic_any.
        make_variadic_scalar("to_json", DataType::Utf8),
        make_variadic_scalar("map_from_entries", DataType::Utf8),
        make_variadic_scalar("json_keys", DataType::Utf8),
        // The standardization stage SQL (stg/int_std assembly, er-entity-v0.1.3):
        // JSON payload accessors, list shaping (the source_metadata lambdas are
        // rewritten to lambda-free forms by the analyser before planning),
        // unicode normalizers, and the date/regex validators. Loose variadic
        // signatures — the stage passes check names/shape, the oracle types.
        make_variadic_scalar("json_extract_string", DataType::Utf8),
        make_variadic_scalar("json_extract", DataType::Utf8),
        make_variadic_scalar("list_contains", DataType::Boolean),
        make_variadic_scalar("list_distinct", list_of_utf8()),
        make_variadic_scalar("list_sort", list_of_utf8()),
        make_variadic_scalar("list_filter", list_of_utf8()),
        make_variadic_scalar("list_transform", list_of_utf8()),
        make_variadic_scalar("list_prepend", list_of_utf8()),
        make_variadic_scalar("nfc_normalize", DataType::Utf8),
        make_variadic_scalar("strip_accents", DataType::Utf8),
        make_variadic_scalar("regexp_full_match", DataType::Boolean),
        make_variadic_scalar(
            "try_strptime",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        make_variadic_scalar("array_length", DataType::Int64),
        make_scalar(
            "now",
            vec![],
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
        ),
        make_variadic_scalar("concat", DataType::Utf8),
        make_variadic_scalar("concat_ws", DataType::Utf8),
        // Map/list element access, e.g. `element_at(m, key)` — the analyser's
        // target when rewriting DuckDB `m[key].field` subscript-field access.
        make_variadic_scalar("element_at", DataType::Utf8),
        make_scalar(
            "struct_extract",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Utf8,
        ),
        make_variadic_scalar("list_value", DataType::Utf8),
        make_scalar(
            "list_extract",
            vec![DataType::Utf8, DataType::Int64],
            DataType::Utf8,
        ),
        make_scalar("unnest", vec![DataType::Utf8], DataType::Utf8),
        make_scalar(
            "generate_series",
            vec![DataType::Int64, DataType::Int64],
            DataType::Int64,
        ),
        make_scalar("hash", vec![DataType::Utf8], DataType::Int64),
        make_scalar("md5", vec![DataType::Utf8], DataType::Utf8),
        make_scalar("sha256", vec![DataType::Utf8], DataType::Utf8),
        make_scalar(
            "format",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Utf8,
        ),
        make_scalar(
            "printf",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Utf8,
        ),
        make_scalar(
            "string_split",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Utf8,
        ),
        make_scalar("lower", vec![DataType::Utf8], DataType::Utf8),
        make_scalar("upper", vec![DataType::Utf8], DataType::Utf8),
        make_scalar("trim", vec![DataType::Utf8], DataType::Utf8),
        make_scalar("ltrim", vec![DataType::Utf8], DataType::Utf8),
        make_scalar("rtrim", vec![DataType::Utf8], DataType::Utf8),
        make_scalar("length", vec![DataType::Utf8], DataType::Int64),
        make_scalar("char_length", vec![DataType::Utf8], DataType::Int64),
        make_scalar("character_length", vec![DataType::Utf8], DataType::Int64),
        make_scalar(
            "replace",
            vec![DataType::Utf8, DataType::Utf8, DataType::Utf8],
            DataType::Utf8,
        ),
        make_scalar(
            "substring",
            vec![DataType::Utf8, DataType::Int64, DataType::Int64],
            DataType::Utf8,
        ),
        make_scalar(
            "left",
            vec![DataType::Utf8, DataType::Int64],
            DataType::Utf8,
        ),
        make_scalar(
            "right",
            vec![DataType::Utf8, DataType::Int64],
            DataType::Utf8,
        ),
        make_scalar(
            "lpad",
            vec![DataType::Utf8, DataType::Int64, DataType::Utf8],
            DataType::Utf8,
        ),
        make_scalar(
            "rpad",
            vec![DataType::Utf8, DataType::Int64, DataType::Utf8],
            DataType::Utf8,
        ),
        make_scalar("reverse", vec![DataType::Utf8], DataType::Utf8),
        make_scalar(
            "repeat",
            vec![DataType::Utf8, DataType::Int64],
            DataType::Utf8,
        ),
        make_scalar(
            "starts_with",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Boolean,
        ),
        make_scalar(
            "ends_with",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Boolean,
        ),
        make_scalar("ascii", vec![DataType::Utf8], DataType::Int64),
        make_scalar("chr", vec![DataType::Int64], DataType::Utf8),
        make_scalar("bit_length", vec![DataType::Utf8], DataType::Int64),
        make_scalar("octet_length", vec![DataType::Utf8], DataType::Int64),
        make_scalar(
            "position",
            vec![DataType::Utf8, DataType::Utf8],
            DataType::Int64,
        ),
        make_scalar(
            "translate",
            vec![DataType::Utf8, DataType::Utf8, DataType::Utf8],
            DataType::Utf8,
        ),
    ]
}

/// Register all DuckDB aggregate UDFs
///
/// Includes standard SQL aggregates (SUM, AVG, etc.) that DataFusion's
/// planning-only context does not provide by default, plus DuckDB-specific ones.
pub(crate) fn duckdb_aggregate_udfs() -> Vec<Arc<AggregateUDF>> {
    vec![
        make_type_preserving_aggregate("sum"),
        make_type_preserving_aggregate("avg"),
        make_type_preserving_aggregate("min"),
        make_type_preserving_aggregate("max"),
        make_aggregate("count", DataType::Utf8, DataType::Int64),
        make_aggregate("string_agg", DataType::Utf8, DataType::Utf8),
        make_aggregate("group_concat", DataType::Utf8, DataType::Utf8),
        make_aggregate("list_agg", DataType::Utf8, DataType::Utf8),
        // DuckDB's `list(expr [ORDER BY ...])` aggregate (alias of list_agg/array_agg)
        // — used by the golden-mart metadata assembly.
        make_aggregate("list", DataType::Utf8, DataType::Utf8),
        make_aggregate("array_agg", DataType::Utf8, DataType::Utf8),
        make_aggregate("bool_and", DataType::Boolean, DataType::Boolean),
        make_aggregate("bool_or", DataType::Boolean, DataType::Boolean),
        make_aggregate("every", DataType::Boolean, DataType::Boolean),
        make_aggregate("listagg", DataType::Utf8, DataType::Utf8),
        make_aggregate("approx_count_distinct", DataType::Utf8, DataType::Int64),
        make_aggregate("approx_quantile", DataType::Float64, DataType::Float64),
        make_aggregate("median", DataType::Float64, DataType::Float64),
        make_aggregate("mode", DataType::Utf8, DataType::Utf8),
        make_aggregate("arg_min", DataType::Utf8, DataType::Utf8),
        make_aggregate("arg_max", DataType::Utf8, DataType::Utf8),
    ]
}

/// Register all DuckDB / SQL-standard window UDFs.
///
/// DataFusion's planning-only context provides no built-in window functions
/// (they live in `datafusion-functions-window`, which this crate does not link),
/// so the ranking and navigation window functions are registered here as stubs.
/// Ranking functions (`row_number`, `rank`, …) type as `BIGINT`; navigation
/// functions (`lag`, `lead`, `first_value`, …) preserve their first argument's
/// type. Like the scalar/aggregate stubs, they are never executed.
pub(crate) fn duckdb_window_udfs() -> Vec<Arc<WindowUDF>> {
    vec![
        make_window("row_number", Some(DataType::Int64)),
        make_window("rank", Some(DataType::Int64)),
        make_window("dense_rank", Some(DataType::Int64)),
        make_window("ntile", Some(DataType::Int64)),
        make_window("percent_rank", Some(DataType::Float64)),
        make_window("cume_dist", Some(DataType::Float64)),
        // Navigation functions return a value drawn from their first argument.
        make_window("lag", None),
        make_window("lead", None),
        make_window("first_value", None),
        make_window("last_value", None),
        make_window("nth_value", None),
    ]
}

/// Create a window UDF stub. `ret == Some(dt)` fixes the output type (ranking
/// functions); `ret == None` makes it type-preserving over the first argument
/// (navigation functions like `lag`/`first_value`).
fn make_window(name: &str, ret: Option<DataType>) -> Arc<WindowUDF> {
    // Accept both zero arguments (ranking functions: `row_number()`) and any
    // positional arguments (navigation functions: `lag(col, 1)`, `ntile(4)`).
    let signature = Signature::new(
        TypeSignature::OneOf(vec![TypeSignature::Nullary, TypeSignature::VariadicAny]),
        Volatility::Immutable,
    );
    Arc::new(WindowUDF::from(StubWindowUDF {
        name: name.to_string(),
        signature,
        return_type: ret,
    }))
}

/// Create a scalar UDF with exact signature

/// `LIST(VARCHAR)` — the list type the standardization stage's list-shaping
/// stubs return, so CASE arms against `cast([] as varchar[])` coerce.
fn list_of_utf8() -> DataType {
    DataType::List(std::sync::Arc::new(arrow::datatypes::Field::new(
        "item",
        DataType::Utf8,
        true,
    )))
}

fn make_scalar(name: &str, args: Vec<DataType>, ret: DataType) -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::from(StubScalarUDF {
        name: name.to_string(),
        signature: Signature::new(TypeSignature::Exact(args), Volatility::Immutable),
        return_type: ret,
    }))
}

/// Create a scalar UDF with multiple overloaded signatures via `TypeSignature::OneOf`
fn make_overloaded_scalar(name: &str, overloads: &[(Vec<DataType>, DataType)]) -> Arc<ScalarUDF> {
    let sigs: Vec<TypeSignature> = overloads
        .iter()
        .map(|(args, _)| TypeSignature::Exact(args.clone()))
        .collect();
    let ret = overloads
        .first()
        .map(|(_, r)| r.clone())
        .unwrap_or(DataType::Utf8);
    Arc::new(ScalarUDF::from(StubScalarUDF {
        name: name.to_string(),
        signature: Signature::new(TypeSignature::OneOf(sigs), Volatility::Immutable),
        return_type: ret,
    }))
}

/// Create a variadic scalar UDF
fn make_variadic_scalar(name: &str, ret: DataType) -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::from(StubScalarUDF {
        name: name.to_string(),
        signature: Signature::variadic_any(Volatility::Immutable),
        return_type: ret,
    }))
}

/// A variadic text-returning scalar stub, for the custom `ExprPlanner` to wrap
/// constructs DataFusion has no builder for (DuckDB `MAP {..}` literals, map/struct
/// subscript field access): the wrapped argument expressions are preserved so their
/// column references are still resolved, and the result types as text.
pub(crate) fn variadic_text_stub(name: &str) -> Arc<ScalarUDF> {
    make_variadic_scalar(name, DataType::Utf8)
}

/// A variadic LIST(VARCHAR)-returning scalar stub, for the custom `ExprPlanner`
/// to wrap DuckDB array literals (`['a', 'b']`, `[]`): the element expressions
/// are preserved so their column references resolve, and the result carries a
/// list type so casts (`[]::varchar[]`) and list functions coerce.
pub(crate) fn variadic_list_stub(name: &str) -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::from(StubScalarUDF {
        name: name.to_string(),
        // Zero args allowed: the empty array literal `[]` (always cast at the
        // use site, e.g. `[]::varchar[]`) plans through this stub too.
        signature: Signature::one_of(
            vec![TypeSignature::Nullary, TypeSignature::VariadicAny],
            Volatility::Immutable,
        ),
        return_type: list_of_utf8(),
    }))
}

/// Create a type-preserving variadic scalar UDF (returns first non-Null arg type)
fn make_type_preserving_variadic_scalar(name: &str) -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::from(TypePreservingScalarUDF {
        name: name.to_string(),
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
}

/// Create a type-preserving aggregate UDF (return type matches input type)
fn make_type_preserving_aggregate(name: &str) -> Arc<AggregateUDF> {
    Arc::new(AggregateUDF::from(TypePreservingAggregateUDF {
        name: name.to_string(),
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
}

/// Create an aggregate UDF
fn make_aggregate(name: &str, input: DataType, ret: DataType) -> Arc<AggregateUDF> {
    Arc::new(AggregateUDF::from(StubAggregateUDF {
        name: name.to_string(),
        signature: Signature::new(TypeSignature::Variadic(vec![input]), Volatility::Immutable),
        return_type: ret,
    }))
}

/// Create a scalar UDF stub from user-defined function metadata.
///
/// Converts SQL type strings to Arrow types and registers a stub with the given
/// name and signature. This allows DataFusion to plan queries that call user-defined
/// functions without actually executing them.
pub(crate) fn make_user_scalar_udf(
    name: &str,
    arg_types: &[String],
    return_type: &str,
) -> Arc<ScalarUDF> {
    use crate::datafusion_bridge::types::sql_type_to_arrow;
    use crate::types::parse_sql_type;

    let arrow_args: Vec<DataType> = arg_types
        .iter()
        .map(|t| sql_type_to_arrow(&parse_sql_type(t)))
        .collect();
    let arrow_ret = sql_type_to_arrow(&parse_sql_type(return_type));

    make_scalar(name, arrow_args, arrow_ret)
}

/// A no-op scalar UDF stub that provides type information for DataFusion planning.
///
/// These stubs are never executed; they exist solely so that DataFusion's
/// `SqlToRel` planner can resolve DuckDB-specific scalar functions and infer
/// their return types during static analysis.
#[derive(Debug, Hash, PartialEq, Eq)]
struct StubScalarUDF {
    name: String,
    signature: Signature,
    return_type: DataType,
}

impl ScalarUDFImpl for StubScalarUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _args: &[DataType]) -> DFResult<DataType> {
        Ok(self.return_type.clone())
    }

    fn invoke_with_args(
        &self,
        _args: datafusion_expr::ScalarFunctionArgs,
    ) -> DFResult<ColumnarValue> {
        Err(DataFusionError::Internal(format!(
            "Stub scalar UDF '{}' should not be executed during static analysis",
            self.name
        )))
    }
}

/// A no-op aggregate UDF stub that provides type information for DataFusion planning.
///
/// These stubs are never executed; they exist solely so that DataFusion's
/// `SqlToRel` planner can resolve DuckDB-specific aggregate functions and infer
/// their return types during static analysis.
#[derive(Debug, Hash, PartialEq, Eq)]
struct StubAggregateUDF {
    name: String,
    signature: Signature,
    return_type: DataType,
}

impl datafusion_expr::AggregateUDFImpl for StubAggregateUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _args: &[DataType]) -> DFResult<DataType> {
        Ok(self.return_type.clone())
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> DFResult<Box<dyn Accumulator>> {
        Err(DataFusionError::Internal(format!(
            "Stub aggregate UDF '{}' should not be executed during static analysis",
            self.name
        )))
    }
}

/// A scalar UDF that returns the type of its first non-Null argument.
///
/// Used for COALESCE, IFNULL, NULLIF where the output type should match the input.
#[derive(Debug, Hash, PartialEq, Eq)]
struct TypePreservingScalarUDF {
    name: String,
    signature: Signature,
}

impl ScalarUDFImpl for TypePreservingScalarUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, args: &[DataType]) -> DFResult<DataType> {
        Ok(args
            .iter()
            .find(|t| !matches!(t, DataType::Null))
            .cloned()
            .unwrap_or(DataType::Utf8))
    }

    fn invoke_with_args(
        &self,
        _args: datafusion_expr::ScalarFunctionArgs,
    ) -> DFResult<ColumnarValue> {
        Err(DataFusionError::Internal(format!(
            "Type-preserving stub scalar UDF '{}' should not be executed during static analysis",
            self.name
        )))
    }
}

/// An aggregate UDF that returns the same type as its input.
///
/// Used for SUM, AVG, MIN, MAX where the output type should match the input.
#[derive(Debug, Hash, PartialEq, Eq)]
struct TypePreservingAggregateUDF {
    name: String,
    signature: Signature,
}

impl datafusion_expr::AggregateUDFImpl for TypePreservingAggregateUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, args: &[DataType]) -> DFResult<DataType> {
        Ok(args.first().cloned().unwrap_or(DataType::Utf8))
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> DFResult<Box<dyn Accumulator>> {
        Err(DataFusionError::Internal(format!(
            "Type-preserving stub aggregate UDF '{}' should not be executed during static analysis",
            self.name
        )))
    }
}

/// A no-op window UDF stub that provides type information for DataFusion planning.
///
/// These stubs are never executed; they exist solely so that DataFusion's
/// `SqlToRel` planner can resolve window functions (`row_number()`, `rank()`,
/// `lag()`, …) and infer their output type during static analysis.
#[derive(Debug, Hash, PartialEq, Eq)]
struct StubWindowUDF {
    name: String,
    signature: Signature,
    /// `Some(dt)` for a fixed output type (ranking functions); `None` to preserve
    /// the first argument's type (navigation functions).
    return_type: Option<DataType>,
}

impl WindowUDFImpl for StubWindowUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn partition_evaluator(
        &self,
        _args: PartitionEvaluatorArgs,
    ) -> DFResult<Box<dyn PartitionEvaluator>> {
        Err(DataFusionError::Internal(format!(
            "Stub window UDF '{}' should not be executed during static analysis",
            self.name
        )))
    }

    fn field(&self, field_args: WindowUDFFieldArgs) -> DFResult<FieldRef> {
        let data_type = match &self.return_type {
            Some(dt) => dt.clone(),
            None => field_args
                .get_input_field(0)
                .map(|f| f.data_type().clone())
                .unwrap_or(DataType::Int64),
        };
        Ok(Arc::new(Field::new(field_args.name(), data_type, true)))
    }
}
