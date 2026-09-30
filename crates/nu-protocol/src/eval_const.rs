//! Implementation of const-evaluation
//!
//! This enables you to assign `const`-constants and execute parse-time code dependent on this.
//! e.g. `source $my_const`
use crate::{
    BlockId, Config, HistoryPath, IN_VARIABLE_ID, PipelineData, Record, ShellError, Signals, Span,
    Value, VarId,
    ast::{self, Assignment, Block, Call, Expr, Expression, ExternalArgument},
    debugger::{DebugContext, WithoutDebug},
    engine::{
        Argument, Closure, CommandType, EngineState, Stack, StateWorkingSet,
        named_flags::normalize_engine_arguments,
    },
    eval_base::Eval,
    ir, record,
    shell_error::generic::GenericError,
};
use nu_system::os_info::{get_kernel_version, get_os_arch, get_os_family, get_os_name};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Create a Value for `$nu`.
// Note: When adding new constants to $nu, please update the doc at https://nushell.sh/book/special_variables.html
// or at least add a TODO/reminder issue in nushell.github.io so we don't lose track of it.
pub(crate) fn create_nu_constant(engine_state: &EngineState, span: Span) -> Value {
    fn canonicalize_path(engine_state: &EngineState, path: &Path) -> PathBuf {
        #[allow(deprecated)]
        let cwd = engine_state.current_work_dir();

        if path.exists() {
            match nu_path::canonicalize_with(path, cwd) {
                Ok(canon_path) => canon_path,
                Err(_) => path.to_owned(),
            }
        } else {
            path.to_owned()
        }
    }

    let mut record = Record::new();

    let config_home = &engine_state.config_dirs.config_home;
    let canon_config_home = canonicalize_path(engine_state, config_home);

    record.push(
        "default-config-dir",
        Value::string(canon_config_home.to_string_lossy(), span),
    );

    record.push(
        "config-path",
        Value::string(
            canonicalize_path(engine_state, engine_state.config_dirs.config_file.as_path())
                .to_string_lossy(),
            span,
        ),
    );

    record.push(
        "env-path",
        Value::string(
            canonicalize_path(engine_state, engine_state.config_dirs.env_file.as_path())
                .to_string_lossy(),
            span,
        ),
    );

    record.push(
        "history-path",
        match &engine_state.config.history.path {
            HistoryPath::Disabled => Value::string("", span),
            HistoryPath::Custom(custom_path) => {
                let effective_path = if custom_path.is_dir() {
                    custom_path.join(engine_state.config.history.file_format.default_file_name())
                } else {
                    custom_path.clone()
                };
                let canon_hist_path = canonicalize_path(engine_state, &effective_path);
                Value::string(canon_hist_path.to_string_lossy(), span)
            }
            HistoryPath::Default => {
                // Use the same resolution path as history backends so `$nu.history-path`
                // always matches the file reedline opens.
                let hist_path = engine_state
                    .config
                    .history
                    .file_path(config_home)
                    .unwrap_or_else(|| {
                        config_home
                            .join(engine_state.config.history.file_format.default_file_name())
                    });
                let canon_hist_path = canonicalize_path(engine_state, &hist_path);
                Value::string(canon_hist_path.to_string_lossy(), span)
            }
        },
    );

    record.push(
        "loginshell-path",
        Value::string(
            canonicalize_path(engine_state, &config_home.join("login.nu")).to_string_lossy(),
            span,
        ),
    );

    #[cfg(feature = "plugin")]
    {
        // Prefer the live plugin_path (set once at startup from config_dirs).
        let plugin_path = engine_state
            .plugin_path
            .as_deref()
            .unwrap_or_else(|| engine_state.config_dirs.plugin_file.as_path());
        record.push(
            "plugin-path",
            Value::string(
                canonicalize_path(engine_state, plugin_path).to_string_lossy(),
                span,
            ),
        );
    }

    let home_dir = &engine_state.config_dirs.home_dir;
    record.push(
        "home-dir",
        if home_dir.as_os_str().is_empty() {
            Value::error(
                ShellError::Generic(GenericError::new(
                    "setting $nu.home-dir failed",
                    "Could not get home directory",
                    span,
                )),
                span,
            )
        } else {
            Value::string(
                canonicalize_path(engine_state, home_dir).to_string_lossy(),
                span,
            )
        },
    );

    record.push(
        "data-dir",
        Value::string(
            canonicalize_path(engine_state, &engine_state.config_dirs.data_home).to_string_lossy(),
            span,
        ),
    );

    record.push(
        "cache-dir",
        Value::string(
            canonicalize_path(engine_state, &engine_state.config_dirs.cache_home).to_string_lossy(),
            span,
        ),
    );

    record.push(
        "vendor-autoload-dirs",
        Value::list(
            engine_state
                .config_dirs
                .vendor_autoload_dirs
                .iter()
                .map(|path| Value::string(path.to_string_lossy(), span))
                .collect(),
            span,
        ),
    );

    record.push(
        "user-autoload-dirs",
        Value::list(
            engine_state
                .config_dirs
                .user_autoload_dirs
                .iter()
                .map(|path| Value::string(path.to_string_lossy(), span))
                .collect(),
            span,
        ),
    );

    record.push("temp-dir", {
        let canon_temp_path = canonicalize_path(engine_state, &std::env::temp_dir());
        Value::string(canon_temp_path.to_string_lossy(), span)
    });

    record.push("pid", Value::int(std::process::id().into(), span));

    record.push("os-info", {
        let ver = get_kernel_version();
        Value::record(
            record! {
                "name" => Value::string(get_os_name(), span),
                "arch" => Value::string(get_os_arch(), span),
                "family" => Value::string(get_os_family(), span),
                "kernel_version" => Value::string(ver, span),
            },
            span,
        )
    });

    record.push(
        "startup-time",
        Value::duration(engine_state.get_startup_time(), span),
    );

    record.push(
        "is-interactive",
        Value::bool(engine_state.is_interactive, span),
    );

    record.push("is-login", Value::bool(engine_state.is_login, span));

    record.push(
        "history-enabled",
        Value::bool(engine_state.history_enabled, span),
    );

    record.push(
        "current-exe",
        if let Ok(current_exe) = std::env::current_exe() {
            Value::string(current_exe.to_string_lossy(), span)
        } else {
            Value::error(
                ShellError::Generic(GenericError::new(
                    "setting $nu.current-exe failed",
                    "Could not get current executable path",
                    span,
                )),
                span,
            )
        },
    );

    record.push("is-lsp", Value::bool(engine_state.is_lsp, span));
    record.push("is-mcp", Value::bool(engine_state.is_mcp, span));
    record.push("is-dap", Value::bool(engine_state.is_dap, span));

    Value::record(record, span)
}

/// Operation budget for a single top-level const evaluation.
///
/// Const evaluation may run user code (custom commands, loops). The fuel counter
/// bounds total work so a non-terminating `const` program is a parse-time error
/// rather than a hung parser.
const CONST_EVAL_FUEL: usize = 10_000_000;

/// Fallback custom-command call depth for const evaluation when the configured
/// `recursion_limit` is unusable. Const-eval recursion nests Rust calls deeply,
/// so depth is capped like the runtime's.
const CONST_EVAL_FALLBACK_DEPTH: usize = 50;

/// Mutable environment threaded through a const evaluation.
///
/// `const` bindings continue to live on the `Variable` itself (`const_val`). This
/// environment covers the additional state produced by calling custom commands at
/// parse time: parameter bindings, `let`/`mut` bindings inside def bodies, and
/// `for`/`while` loop variables.
#[derive(Debug)]
pub struct ConstEvalEnv {
    /// Variable bindings, in binding order. Lookups scan from the back so that
    /// later bindings shadow earlier ones naturally.
    bindings: Vec<(VarId, Value)>,
    /// Remaining operation budget. Each pipeline element and loop iteration spends fuel.
    fuel: usize,
    /// Current custom-command call depth.
    depth: usize,
}

impl Default for ConstEvalEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstEvalEnv {
    pub fn new() -> Self {
        Self {
            bindings: vec![],
            fuel: CONST_EVAL_FUEL,
            depth: 0,
        }
    }

    fn enter_call(&mut self, limit: u64, span: Span) -> Result<(), ShellError> {
        if self.depth as u64 >= limit {
            return Err(ShellError::RecursionLimitReached {
                recursion_limit: limit,
                span: Some(span),
            });
        }
        self.depth += 1;
        Ok(())
    }

    fn get(&self, var_id: VarId) -> Option<&Value> {
        self.bindings
            .iter()
            .rev()
            .find(|(id, _)| *id == var_id)
            .map(|(_, val)| val)
    }

    fn assign(&mut self, var_id: VarId, value: Value) -> bool {
        if let Some((_, slot)) = self.bindings.iter_mut().rev().find(|(id, _)| *id == var_id) {
            *slot = value;
            true
        } else {
            false
        }
    }

    fn spend(&mut self, span: Span) -> Result<(), ShellError> {
        self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
            ShellError::Generic(GenericError::new(
                "Const evaluation exceeded the operation limit",
                "This `const` expression required more work than const evaluation allows \
                     (loops and recursion are bounded).",
                span,
            ))
        })?;
        Ok(())
    }
}

fn eval_const_call(
    working_set: &StateWorkingSet,
    call: &Call,
    input: PipelineData,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let decl = working_set.get_decl(call.decl_id);

    // Keywords that need AST structure; IR-shaped flat Values are not enough.
    if decl.command_type() == CommandType::Keyword {
        match decl.name() {
            "if" => return eval_const_if(working_set, call, input, env),
            // parse_const already set const_val; AST args include VarDecl (not a Value).
            "const" => return Ok(PipelineData::empty()),
            "let" | "mut" => return eval_const_let_mut(working_set, call, env),
            "for" => return eval_const_for(working_set, call, env),
            "while" => return eval_const_while(working_set, call, env),
            _ => {}
        }
    }

    // Custom commands (defs): bind the call arguments to the body's parameter
    // variables and evaluate the block. The body is evaluated with the same
    // const-evaluation machinery, so calls to non-const commands inside it fail
    // with `NotAConstCommand` naturally.
    if let Some(block_id) = decl.block_id() {
        return eval_const_def_call(working_set, call, block_id, input, env);
    }

    if !decl.is_const() {
        return Err(ShellError::NotAConstCommand { span: call.head });
    }

    if !decl.is_known_external() && call.named_iter().any(|(flag, _, _)| flag.item == "help") {
        // It would require re-implementing get_full_help() for const evaluation. Assuming that
        // getting help messages at parse-time is rare enough, we can simply disallow it.
        return Err(ShellError::NotAConstHelp { span: call.head });
    }

    let mut stack = Stack::new();
    let ir_call = build_const_ir_call(working_set, call, &decl.signature(), &mut stack, env)?;
    let result = decl.run_const(working_set, &mut stack, &(&ir_call).into(), input);
    ir_call.leave(&mut stack);
    result
}

/// Const evaluation of a custom command call (`def`).
///
/// Replicates the runtime `gather_arguments` binding semantics: positional
/// arguments bind to the signature's positional var ids, leftovers collect into
/// the rest parameter, named arguments bind to their var ids, and unset
/// parameters get their declared default (or `null`/`false`). The body's input
/// is also bound to `$in` so defs can read it explicitly.
fn eval_const_def_call(
    working_set: &StateWorkingSet,
    call: &Call,
    block_id: BlockId,
    input: PipelineData,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let limit = working_set
        .get_config()
        .recursion_limit
        .try_into()
        .unwrap_or(CONST_EVAL_FALLBACK_DEPTH as u64);
    env.enter_call(limit, call.head)?;
    let result = eval_const_def_call_inner(working_set, call, block_id, input, env);
    env.depth -= 1;
    result
}

fn eval_const_def_call_inner(
    working_set: &StateWorkingSet,
    call: &Call,
    block_id: BlockId,
    input: PipelineData,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let block = working_set.get_block(block_id);

    let mut stack = Stack::new();
    let ir_call = build_const_ir_call(working_set, call, &block.signature, &mut stack, env)?;
    let args: Vec<Argument> = stack
        .arguments
        .drain_args(ir_call.args_base, ir_call.args_len)
        .collect();
    ir_call.leave(&mut stack);

    let checkpoint = env.bindings.len();
    let result = bind_const_args(&block.signature, args, env, call.head).and_then(|_| {
        // `def`s don't capture `const` bindings the way closures capture vars, but
        // `$in` reads the pipeline input like at runtime.
        if let PipelineData::Value(v, ..) = &input {
            env.bindings.push((IN_VARIABLE_ID, v.clone()));
        }
        eval_const_subexpression_inner(working_set, block, input, call.head, env)
    });
    env.bindings.truncate(checkpoint);
    result
}

/// Bind normalized call arguments to parameter variables. Mirrors
/// `gather_arguments` in the runtime evaluator, targeting [`ConstEvalEnv`].
fn bind_const_args(
    signature: &crate::Signature,
    args: Vec<Argument>,
    env: &mut ConstEvalEnv,
    call_head: Span,
) -> Result<(), ShellError> {
    let find_named = |data: &Arc<[u8]>, name: ir::DataSlice, short: ir::DataSlice, span: Span| {
        signature
            .named
            .iter()
            .find(|n| match (n.long_name(), n.short) {
                (Some(long), _) => long.as_bytes() == &data[name],
                (None, Some(s)) => s.encode_utf8(&mut [0; 4]).as_bytes() == &data[short],
                (None, None) => false,
            })
            .and_then(|f| f.var_id)
            .ok_or_else(|| ShellError::NotAConstant { span })
    };
    let positional_var_id = |arg: &crate::PositionalArg, span: Span| {
        arg.var_id.ok_or(ShellError::NotAConstant { span })
    };

    let mut positional_iter = signature
        .required_positional
        .iter()
        .chain(signature.optional_positional.iter());

    let mut rest = vec![];
    let mut rest_span: Option<Span> = None;
    let mut always_spread = false;

    for arg in args {
        match arg {
            Argument::Positional { span, val, .. } => {
                match (!always_spread).then(|| positional_iter.next()).flatten() {
                    Some(positional_arg) => {
                        let var_id = positional_var_id(positional_arg, span)?;
                        // The parser already checked call-site types.
                        env.bindings.push((var_id, val));
                    }
                    None => {
                        rest_span = Some(rest_span.map_or(span, |s| s.append(span)));
                        rest.push(val);
                    }
                }
            }
            Argument::Spread {
                vals,
                span: spread_span,
                ..
            } => match vals {
                Value::List { vals, .. } => {
                    rest.extend(vals);
                    rest_span = Some(rest_span.map_or(spread_span, |s| s.append(spread_span)));
                    always_spread = true;
                }
                Value::Nothing { .. } => {
                    rest_span = Some(rest_span.map_or(spread_span, |s| s.append(spread_span)));
                    always_spread = true;
                }
                Value::Error { error, .. } => return Err(*error),
                _ => return Err(ShellError::CannotSpreadAsList { span: vals.span() }),
            },
            Argument::Flag {
                data,
                name,
                short,
                span,
            } => {
                let var_id = find_named(&data, name, short, span)?;
                env.bindings.push((var_id, Value::bool(true, span)));
            }
            Argument::Named {
                data,
                name,
                short,
                span,
                val,
                ..
            } => {
                let var_id = find_named(&data, name, short, span)?;
                env.bindings.push((var_id, val));
            }
            Argument::ParserInfo { .. } => (),
        }
    }

    finish_const_args(signature, positional_iter, rest, rest_span, env, call_head)
}

/// Shared tail of argument binding: collect leftovers into the rest parameter,
/// fill unfilled positionals with their declared default (or `null`), and fill
/// unset named parameters with their default (or `null`/`false`).
fn finish_const_args<'a>(
    signature: &'a crate::Signature,
    positional_iter: impl Iterator<Item = &'a crate::PositionalArg>,
    rest: Vec<Value>,
    rest_span: Option<Span>,
    env: &mut ConstEvalEnv,
    call_head: Span,
) -> Result<(), ShellError> {
    let positional_var_id = |arg: &crate::PositionalArg, span: Span| {
        arg.var_id.ok_or(ShellError::NotAConstant { span })
    };

    if let Some(rest_arg) = &signature.rest_positional {
        let rest_span = rest_span.unwrap_or(call_head);
        let var_id = positional_var_id(rest_arg, rest_span)?;
        env.bindings.push((var_id, Value::list(rest, rest_span)));
    }

    for positional_arg in positional_iter {
        let var_id = positional_var_id(positional_arg, call_head)?;
        env.bindings.push((
            var_id,
            positional_arg
                .default_value
                .clone()
                .unwrap_or_else(|| Value::nothing(call_head)),
        ));
    }

    for named_arg in &signature.named {
        if let Some(var_id) = named_arg.var_id
            && !env.bindings.iter().any(|(id, _)| *id == var_id)
        {
            let val = if named_arg.arg.is_none() {
                Value::bool(false, call_head)
            } else {
                named_arg
                    .default_value
                    .clone()
                    .unwrap_or_else(|| Value::nothing(call_head))
            };
            env.bindings.push((var_id, val));
        }
    }

    Ok(())
}

/// Evaluate a [`Value::Closure`] at const-eval time: bind its captured variables
/// (resolving `const` bindings when the parser has not filled `captures` yet),
/// bind positional arguments, and evaluate the block. This is the const-eval
/// counterpart of the runtime's `ClosureEval` for const-marked commands such as
/// `each`.
///
/// The closure's free variables resolve through `const` values; closures
/// capturing `let`/`mut` locals fail with `NotAConstant`, matching the existing
/// limitation that captures are not known during const evaluation.
pub fn eval_const_closure(
    working_set: &StateWorkingSet,
    closure: &Closure,
    args: Vec<Value>,
    input: PipelineData,
    span: Span,
) -> Result<PipelineData, ShellError> {
    let mut env = ConstEvalEnv::new();
    eval_const_closure_inner(working_set, closure, args, input, span, &mut env)
}

fn eval_const_closure_inner(
    working_set: &StateWorkingSet,
    closure: &Closure,
    args: Vec<Value>,
    input: PipelineData,
    span: Span,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let limit = working_set
        .get_config()
        .recursion_limit
        .try_into()
        .unwrap_or(CONST_EVAL_FALLBACK_DEPTH as u64);
    env.enter_call(limit, span)?;

    let block = working_set.get_block(closure.block_id);
    let checkpoint = env.bindings.len();

    let result = (|| {
        // Bind captures resolved by the parser (usually empty at parse time; free
        // variables then resolve through `const` values instead).
        for (var_id, val) in &closure.captures {
            env.bindings.push((*var_id, val.clone()));
        }

        let mut positional_iter = block
            .signature
            .required_positional
            .iter()
            .chain(block.signature.optional_positional.iter());

        let mut rest = vec![];
        let mut rest_span: Option<Span> = None;
        for val in args {
            match positional_iter.next() {
                Some(positional_arg) => {
                    let var_id = positional_arg
                        .var_id
                        .ok_or(ShellError::NotAConstant { span })?;
                    env.bindings.push((var_id, val));
                }
                None => {
                    rest_span = Some(rest_span.map_or(span, |s| s.append(span)));
                    rest.push(val);
                }
            }
        }

        finish_const_args(
            &block.signature,
            positional_iter,
            rest,
            rest_span,
            env,
            span,
        )?;

        // Like `run_with_value`, the input is also bound to `$in`.
        if let PipelineData::Value(v, ..) = &input {
            env.bindings.push((IN_VARIABLE_ID, v.clone()));
        }
        eval_const_subexpression_inner(working_set, block, input, span, env)
    })();

    env.bindings.truncate(checkpoint);
    env.depth -= 1;
    result
}

/// Const evaluation of `let`/`mut`: evaluate the rvalue block and bind the
/// declared variable in the environment.
fn eval_const_let_mut(
    working_set: &StateWorkingSet,
    call: &Call,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let mut iter = call.positional_iter();
    let var_id = iter
        .next()
        .and_then(|expr| expr.as_var())
        .ok_or(ShellError::NotAConstant { span: call.head })?;
    let rvalue = iter
        .next()
        .ok_or(ShellError::NotAConstant { span: call.head })?;

    let value = eval_const_rvalue(working_set, rvalue, env)?;
    env.bindings.push((var_id, value));
    Ok(PipelineData::empty())
}

/// Evaluate a `let`/`mut` rvalue, which the parser stores as `Expr::Block`.
fn eval_const_rvalue(
    working_set: &StateWorkingSet,
    expr: &Expression,
    env: &mut ConstEvalEnv,
) -> Result<Value, ShellError> {
    match &expr.expr {
        Expr::Block(block_id) | Expr::Subexpression(block_id) => {
            let block = working_set.get_block(*block_id);
            eval_const_subexpression_inner(
                working_set,
                block,
                PipelineData::empty(),
                expr.span,
                env,
            )?
            .into_value(expr.span)
        }
        _ => eval_constant_inner(working_set, env, expr),
    }
}

/// Const evaluation of `for`. Like the runtime keyword, the body is drained —
/// `for` produces no output; it exists to drive `mut` bindings in const code.
fn eval_const_for(
    working_set: &StateWorkingSet,
    call: &Call,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let mut iter = call.positional_iter();
    let (Some(var_expr), Some(in_expr), Some(block_expr)) = (iter.next(), iter.next(), iter.next())
    else {
        return Err(ShellError::NotAConstant { span: call.head });
    };
    let var_id = var_expr
        .as_var()
        .ok_or(ShellError::NotAConstant { span: call.head })?;
    let in_expr = in_expr
        .as_keyword()
        .ok_or(ShellError::NotAConstant { span: call.head })?;
    let block_id = block_expr
        .as_block()
        .ok_or(ShellError::NotAConstant { span: call.head })?;
    let block = working_set.get_block(block_id);

    // Collect eagerly but bounded by fuel, so `for x in 0..inf` is a parse-time
    // error rather than a hang.
    let in_val = eval_constant_inner(working_set, env, in_expr)?;
    let iter: Box<dyn Iterator<Item = Value>> = match in_val {
        Value::List { vals, .. } => Box::new(vals.into_iter()),
        Value::Range { val, .. } => Box::new(val.into_range_iter(in_expr.span, Signals::empty())),
        _ => return Err(ShellError::NotAConstant { span: in_expr.span }),
    };
    let mut items = vec![];
    for item in iter {
        env.spend(in_expr.span)?;
        items.push(item);
    }

    let checkpoint = env.bindings.len();
    for item in items {
        env.spend(call.head)?;
        env.bindings.push((var_id, item));
        // Drain the body: `for` never produces pipeline output.
        eval_const_subexpression_inner(
            working_set,
            block,
            PipelineData::empty(),
            block.span.unwrap_or(call.head),
            env,
        )?;
        env.bindings.truncate(checkpoint);
    }
    Ok(PipelineData::empty())
}

/// Const evaluation of `while`: evaluate the condition and drain the body until
/// the condition is false. Bounded by the const evaluation fuel.
fn eval_const_while(
    working_set: &StateWorkingSet,
    call: &Call,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let mut iter = call.positional_iter();
    let (Some(cond_expr), Some(block_expr)) = (iter.next(), iter.next()) else {
        return Err(ShellError::NotAConstant { span: call.head });
    };
    let block_id = block_expr
        .as_block()
        .ok_or(ShellError::NotAConstant { span: call.head })?;
    let block = working_set.get_block(block_id);

    while eval_constant_inner(working_set, env, cond_expr)?.as_bool()? {
        env.spend(call.head)?;
        eval_const_subexpression_inner(
            working_set,
            block,
            PipelineData::empty(),
            block.span.unwrap_or(call.head),
            env,
        )?;
    }
    Ok(PipelineData::empty())
}

/// Evaluate a single const-call argument expression to a [`Value`].
///
/// A positional declared as [`SyntaxShape::Block`](crate::SyntaxShape::Block) parses to
/// [`Expr::Block`] and is materialized as a capture-free [`Closure`] so const commands can
/// read its source text (e.g. `attr example`). Captures are not known during const
/// evaluation (`block.captures` is filled at the end of parsing), so the closure must not be
/// executed. Closures and row conditions are deliberately left to [`eval_constant`], which
/// rejects them with `NotAConstant`; materializing them here would let e.g.
/// `const c = (echo {|| $v })` parse and then fail at runtime with a missing capture.
fn eval_const_call_arg(
    working_set: &StateWorkingSet,
    expr: &Expression,
    env: &mut ConstEvalEnv,
) -> Result<Value, ShellError> {
    match &expr.expr {
        // `Expr::Closure`/`Expr::RowCondition` are also materialized so that
        // const-marked commands (e.g. `each`) can call them via
        // [`eval_const_closure`].
        Expr::Block(block_id) | Expr::Closure(block_id) | Expr::RowCondition(block_id) => {
            Ok(Value::closure(
                Closure {
                    block_id: *block_id,
                    captures: vec![],
                },
                expr.span,
            ))
        }
        _ => eval_constant_inner(working_set, env, expr),
    }
}

fn build_const_ir_call(
    working_set: &StateWorkingSet,
    call: &Call,
    signature: &crate::Signature,
    stack: &mut Stack,
    env: &mut ConstEvalEnv,
) -> Result<ir::Call, ShellError> {
    let mut builder = ir::Call::build(call.decl_id, call.head);

    for arg in &call.arguments {
        match arg {
            ast::Argument::Positional(expr) | ast::Argument::Unknown(expr) => {
                let val = eval_const_call_arg(working_set, expr, env)?;
                builder.add_positional(stack, expr.span, val);
            }
            ast::Argument::Spread(expr) => {
                let val = eval_const_call_arg(working_set, expr, env)?;
                builder.add_spread(stack, expr.span, val);
            }
            ast::Argument::Named((long, short, maybe_expr)) => {
                let short_name = short.as_ref().map(|s| s.item.as_str()).unwrap_or("");
                if let Some(expr) = maybe_expr {
                    let val = eval_const_call_arg(working_set, expr, env)?;
                    builder.add_named(stack, &long.item, short_name, arg.span(), val);
                } else {
                    builder.add_flag(stack, &long.item, short_name, arg.span());
                }
            }
        }
    }

    for (name, expr) in &call.parser_info {
        let data: std::sync::Arc<[u8]> = name.as_bytes().into();
        let name_slice = ir::DataSlice {
            start: 0,
            len: name.len().try_into().expect("parser info name too big"),
        };
        builder.add_argument(
            stack,
            Argument::ParserInfo {
                data,
                name: name_slice,
                info: Box::new(expr.clone()),
            },
        );
    }

    let mut ir_call = builder.finish();
    // Match runtime IR: expand record flag spreads and omit null named args that
    // do not accept `nothing`.
    let raw: Vec<Argument> = stack
        .arguments
        .drain_args(ir_call.args_base, ir_call.args_len)
        .collect();
    let expanded = normalize_engine_arguments(signature, raw)?;
    ir_call.args_len = expanded.len();
    for arg in expanded {
        stack.arguments.push(arg);
    }

    Ok(ir_call)
}

/// Const evaluation of `if` using AST structure (not IR-shaped call args).
fn eval_const_if(
    working_set: &StateWorkingSet,
    call: &Call,
    input: PipelineData,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    let mut iter = call.positional_iter();
    let cond = iter.next().expect("checked through parser");
    let then_expr = iter.next().expect("checked through parser");
    let else_case = iter.next();

    let then_block = then_expr
        .as_block()
        .ok_or_else(|| ShellError::TypeMismatch {
            err_message: "expected block".into(),
            span: then_expr.span,
        })?;

    if eval_constant_inner(working_set, env, cond)?.as_bool()? {
        let block = working_set.get_block(then_block);
        eval_const_subexpression_inner(
            working_set,
            block,
            input,
            block.span.unwrap_or(call.head),
            env,
        )
    } else if let Some(else_case) = else_case {
        if let Some(else_expr) = else_case.as_keyword() {
            if let Some(block_id) = else_expr.as_block() {
                let block = working_set.get_block(block_id);
                eval_const_subexpression_inner(
                    working_set,
                    block,
                    input,
                    block.span.unwrap_or(call.head),
                    env,
                )
            } else {
                eval_constant_with_input_inner(working_set, else_expr, input, env)
            }
        } else {
            eval_constant_with_input_inner(working_set, else_case, input, env)
        }
    } else {
        Ok(PipelineData::empty())
    }
}

pub fn eval_const_subexpression(
    working_set: &StateWorkingSet,
    block: &Block,
    input: PipelineData,
    span: Span,
) -> Result<PipelineData, ShellError> {
    eval_const_subexpression_inner(working_set, block, input, span, &mut ConstEvalEnv::new())
}

fn eval_const_subexpression_inner(
    working_set: &StateWorkingSet,
    block: &Block,
    mut input: PipelineData,
    span: Span,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    for pipeline in block.pipelines.iter() {
        for element in pipeline.elements.iter() {
            env.spend(span)?;
            if element.redirection.is_some() {
                return Err(ShellError::NotAConstant { span });
            }

            input = eval_constant_with_input_inner(working_set, &element.expr, input, env)?
        }
    }

    Ok(input)
}

pub fn eval_constant_with_input(
    working_set: &StateWorkingSet,
    expr: &Expression,
    input: PipelineData,
) -> Result<PipelineData, ShellError> {
    eval_constant_with_input_inner(working_set, expr, input, &mut ConstEvalEnv::new())
}

fn eval_constant_with_input_inner(
    working_set: &StateWorkingSet,
    expr: &Expression,
    input: PipelineData,
    env: &mut ConstEvalEnv,
) -> Result<PipelineData, ShellError> {
    match &expr.expr {
        Expr::Call(call) => eval_const_call(working_set, call, input, env),
        Expr::Subexpression(block_id) => {
            let block = working_set.get_block(*block_id);
            eval_const_subexpression_inner(working_set, block, input, expr.span(&working_set), env)
        }
        // `$in` usage wraps the element in `Collect`: bind the unique $in variable
        // to the incoming value, then evaluate the inner expression.
        Expr::Collect(var_id, inner) => {
            let in_val = input.into_value(expr.span)?;
            env.bindings.push((*var_id, in_val));
            eval_constant_inner(working_set, env, inner).map(|v| PipelineData::value(v, None))
        }
        _ => eval_constant_inner(working_set, env, expr).map(|v| PipelineData::value(v, None)),
    }
}

/// Evaluate a constant value at parse time
pub fn eval_constant(
    working_set: &StateWorkingSet,
    expr: &Expression,
) -> Result<Value, ShellError> {
    eval_constant_inner(working_set, &mut ConstEvalEnv::new(), expr)
}

fn eval_constant_inner(
    working_set: &StateWorkingSet,
    env: &mut ConstEvalEnv,
    expr: &Expression,
) -> Result<Value, ShellError> {
    // TODO: Allow debugging const eval
    <EvalConst as Eval>::eval::<WithoutDebug>(working_set, env, expr)
}

struct EvalConst;

impl Eval for EvalConst {
    type State<'a> = &'a StateWorkingSet<'a>;

    type MutState = ConstEvalEnv;

    fn get_config(state: Self::State<'_>, _: &mut ConstEvalEnv) -> Arc<Config> {
        state.get_config().clone()
    }

    fn eval_var(
        working_set: &StateWorkingSet,
        env: &mut ConstEvalEnv,
        var_id: VarId,
        span: Span,
    ) -> Result<Value, ShellError> {
        // Bindings from `let`/`mut`, parameters, and loop variables take precedence
        // (they're evaluated state, not parse-time constants).
        if let Some(val) = env.get(var_id) {
            return Ok(val.clone());
        }
        match working_set.get_variable(var_id).const_val.as_ref() {
            Some(val) => Ok(val.clone()),
            None => Err(ShellError::NotAConstant { span }),
        }
    }

    fn eval_call<D: DebugContext>(
        working_set: &StateWorkingSet,
        env: &mut ConstEvalEnv,
        call: &Call,
        span: Span,
    ) -> Result<Value, ShellError> {
        // TODO: Allow debugging const eval
        // TODO: eval.rs uses call.head for the span rather than expr.span
        eval_const_call(working_set, call, PipelineData::empty(), env)?.into_value(span)
    }

    fn eval_external_call(
        _: &StateWorkingSet,
        _: &mut ConstEvalEnv,
        _: &Expression,
        _: &[ExternalArgument],
        span: Span,
    ) -> Result<Value, ShellError> {
        // TODO: It may be more helpful to give not_a_const_command error
        Err(ShellError::NotAConstant { span })
    }

    fn eval_collect<D: DebugContext>(
        _: &StateWorkingSet,
        _: &mut ConstEvalEnv,
        _var_id: VarId,
        expr: &Expression,
    ) -> Result<Value, ShellError> {
        Err(ShellError::NotAConstant { span: expr.span })
    }

    fn eval_subexpression<D: DebugContext>(
        working_set: &StateWorkingSet,
        env: &mut ConstEvalEnv,
        block_id: BlockId,
        span: Span,
    ) -> Result<Value, ShellError> {
        // If parsing errors exist in the subexpression, don't bother to evaluate it.
        if working_set
            .parse_errors
            .iter()
            .any(|error| span.contains_span(error.span()))
        {
            return Err(ShellError::ParseErrorInConstant { span });
        }
        // TODO: Allow debugging const eval
        let block = working_set.get_block(block_id);
        eval_const_subexpression_inner(working_set, block, PipelineData::empty(), span, env)?
            .into_value(span)
    }

    fn regex_match(
        _: &StateWorkingSet,
        _op_span: Span,
        _: &Value,
        _: &Value,
        _: bool,
        expr_span: Span,
    ) -> Result<Value, ShellError> {
        Err(ShellError::NotAConstant { span: expr_span })
    }

    fn eval_assignment<D: DebugContext>(
        working_set: &StateWorkingSet,
        env: &mut ConstEvalEnv,
        lhs: &Expression,
        rhs: &Expression,
        assignment: Assignment,
        op_span: Span,
        expr_span: Span,
    ) -> Result<Value, ShellError> {
        // Only bare-variable assignment is supported in const evaluation.
        let var_id =
            match &lhs.expr {
                Expr::Var(var_id) | Expr::VarDecl(var_id) => *var_id,
                Expr::FullCellPath(cell_path) if cell_path.tail.is_empty() => cell_path
                    .head
                    .as_var()
                    .ok_or(ShellError::NotAConstant { span: expr_span })?,
                _ => return Err(ShellError::NotAConstant { span: expr_span }),
            };

        let rhs = Self::eval::<D>(working_set, env, rhs)?;
        let new_val =
            match assignment {
                Assignment::Assign => rhs,
                Assignment::AddAssign => Self::eval_var(working_set, env, var_id, lhs.span)?
                    .add(op_span, &rhs, expr_span)?,
                Assignment::SubtractAssign => Self::eval_var(working_set, env, var_id, lhs.span)?
                    .sub(op_span, &rhs, expr_span)?,
                Assignment::MultiplyAssign => Self::eval_var(working_set, env, var_id, lhs.span)?
                    .mul(op_span, &rhs, expr_span)?,
                Assignment::DivideAssign => Self::eval_var(working_set, env, var_id, lhs.span)?
                    .div(op_span, &rhs, expr_span)?,
                Assignment::ConcatenateAssign => {
                    Self::eval_var(working_set, env, var_id, lhs.span)?
                        .concat(op_span, &rhs, expr_span)?
                }
            };

        if env.assign(var_id, new_val) {
            Ok(Value::nothing(expr_span))
        } else {
            Err(ShellError::NotAConstant { span: expr_span })
        }
    }

    fn eval_row_condition_or_closure(
        _: &StateWorkingSet,
        _: &mut ConstEvalEnv,
        _: BlockId,
        span: Span,
    ) -> Result<Value, ShellError> {
        Err(ShellError::NotAConstant { span })
    }

    fn eval_overlay(_: &StateWorkingSet, span: Span) -> Result<Value, ShellError> {
        Err(ShellError::NotAConstant { span })
    }

    fn unreachable(working_set: &StateWorkingSet, expr: &Expression) -> Result<Value, ShellError> {
        Err(ShellError::NotAConstant {
            span: expr.span(&working_set),
        })
    }
}
