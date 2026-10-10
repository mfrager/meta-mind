//! The arithmetic leaf of `Term` (`full_system_design.md` §7.2, §15): a
//! self-contained structural representation that Phase 8 bridges into the math
//! subsystem's compiled IR.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

/// The arithmetic leaf of `Term`. Kept symbolic (constants are strings) so the
/// IR stays `Eq`/`Hash`-friendly and exactness is preserved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArithmeticExpr {
    Var(String),
    Const(String),
    Add(Vec<ArithmeticExpr>),
    Sub(Box<ArithmeticExpr>, Box<ArithmeticExpr>),
    Mul(Vec<ArithmeticExpr>),
    Div(Box<ArithmeticExpr>, Box<ArithmeticExpr>),
    Neg(Box<ArithmeticExpr>),
    Pow(Box<ArithmeticExpr>, Box<ArithmeticExpr>),
    /// A named scalar function such as `log2` or `factorial`.
    Function {
        name: String,
        arguments: Vec<ArithmeticExpr>,
    },
    /// Time-shifted series: `(expr, index dimension)`.
    Lag(Box<ArithmeticExpr>, String),
    /// Indexed/panel selection: `(expr, index dimension)`.
    Index(Box<ArithmeticExpr>, String),
}

impl ArithmeticExpr {
    pub fn var(name: impl Into<String>) -> Self {
        ArithmeticExpr::Var(name.into())
    }

    pub fn constant(value: impl Into<String>) -> Self {
        ArithmeticExpr::Const(value.into())
    }

    pub fn add(v: Vec<ArithmeticExpr>) -> Self {
        ArithmeticExpr::Add(v)
    }

    pub fn mul(v: Vec<ArithmeticExpr>) -> Self {
        ArithmeticExpr::Mul(v)
    }

    pub fn subtract(a: ArithmeticExpr, b: ArithmeticExpr) -> Self {
        ArithmeticExpr::Sub(Box::new(a), Box::new(b))
    }

    pub fn divide(a: ArithmeticExpr, b: ArithmeticExpr) -> Self {
        ArithmeticExpr::Div(Box::new(a), Box::new(b))
    }

    pub fn pow(a: ArithmeticExpr, b: ArithmeticExpr) -> Self {
        ArithmeticExpr::Pow(Box::new(a), Box::new(b))
    }

    pub fn negate(e: ArithmeticExpr) -> Self {
        ArithmeticExpr::Neg(Box::new(e))
    }

    /// All variable names referenced.
    pub fn variables(&self, out: &mut BTreeSet<String>) {
        match self {
            ArithmeticExpr::Var(name) => {
                out.insert(name.clone());
            }
            ArithmeticExpr::Const(_) => {}
            ArithmeticExpr::Add(v) | ArithmeticExpr::Mul(v) => {
                for x in v {
                    x.variables(out);
                }
            }
            ArithmeticExpr::Sub(a, b) | ArithmeticExpr::Div(a, b) | ArithmeticExpr::Pow(a, b) => {
                a.variables(out);
                b.variables(out);
            }
            ArithmeticExpr::Function { arguments, .. } => {
                for argument in arguments {
                    argument.variables(out);
                }
            }
            ArithmeticExpr::Neg(a) => a.variables(out),
            ArithmeticExpr::Lag(a, _) | ArithmeticExpr::Index(a, _) => a.variables(out),
        }
    }

    /// Scalar evaluation under an environment (the self-contained evaluator
    /// used by the causal engine; the Numerical IR is the heavier path).
    pub fn eval(&self, env: &HashMap<String, f64>) -> Result<f64, String> {
        match self {
            ArithmeticExpr::Var(name) => env
                .get(name)
                .copied()
                .ok_or_else(|| format!("undefined variable in arithmetic expression: {name}")),
            ArithmeticExpr::Const(c) => c
                .parse::<f64>()
                .map_err(|_| format!("non-numeric constant: {c}")),
            ArithmeticExpr::Add(v) => {
                let mut acc = 0.0;
                for x in v {
                    acc += x.eval(env)?;
                }
                Ok(acc)
            }
            ArithmeticExpr::Sub(a, b) => Ok(a.eval(env)? - b.eval(env)?),
            ArithmeticExpr::Mul(v) => {
                let mut acc = 1.0;
                for x in v {
                    acc *= x.eval(env)?;
                }
                Ok(acc)
            }
            ArithmeticExpr::Div(a, b) => {
                let denom = b.eval(env)?;
                if denom == 0.0 {
                    return Err("division by zero".to_string());
                }
                Ok(a.eval(env)? / denom)
            }
            ArithmeticExpr::Neg(a) => Ok(-a.eval(env)?),
            ArithmeticExpr::Pow(a, b) => Ok(a.eval(env)?.powf(b.eval(env)?)),
            ArithmeticExpr::Function { name, arguments } => {
                let values: Result<Vec<f64>, String> = arguments
                    .iter()
                    .map(|argument| argument.eval(env))
                    .collect();
                let values = values?;
                match (name.as_str(), values.as_slice()) {
                    ("log2", [value]) if *value > 0.0 => Ok(value.log2()),
                    ("factorial", [value]) if *value >= 0.0 && value.fract() == 0.0 => {
                        let mut result = 1.0;
                        for k in 2..=*value as u64 {
                            result *= k as f64;
                        }
                        Ok(result)
                    }
                    ("log2", _) => Err("log2 requires one positive argument".to_string()),
                    ("factorial", _) => {
                        Err("factorial requires one non-negative integer argument".to_string())
                    }
                    _ => Err(format!("unsupported arithmetic function: {name}")),
                }
            }
            ArithmeticExpr::Lag(..) | ArithmeticExpr::Index(..) => {
                Err("series operations require the series runtime, not scalar eval".to_string())
            }
        }
    }

    /// Whether the expression is nonlinear. A product is linear iff at most
    /// one factor is non-constant and that factor is itself linear
    /// (`2·x` is linear; `x·y` is not).
    pub fn is_nonlinear(&self) -> bool {
        match self {
            ArithmeticExpr::Div(..) | ArithmeticExpr::Pow(..) => true,
            ArithmeticExpr::Mul(v) => {
                let non_constant = v
                    .iter()
                    .filter(|x| !matches!(x, ArithmeticExpr::Const(_)))
                    .count();
                non_constant > 1 || v.iter().any(Self::is_nonlinear)
            }
            ArithmeticExpr::Add(v) => v.iter().any(Self::is_nonlinear),
            ArithmeticExpr::Function { .. } => true,
            ArithmeticExpr::Sub(a, b) => a.is_nonlinear() || b.is_nonlinear(),
            ArithmeticExpr::Neg(a) => a.is_nonlinear(),
            ArithmeticExpr::Lag(a, _) | ArithmeticExpr::Index(a, _) => a.is_nonlinear(),
            ArithmeticExpr::Var(_) | ArithmeticExpr::Const(_) => false,
        }
    }
}

impl fmt::Display for ArithmeticExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArithmeticExpr::Var(name) => write!(f, "{name}"),
            ArithmeticExpr::Const(c) => write!(f, "{c}"),
            ArithmeticExpr::Add(v) => write!(f, "({})", join(v, " + ")),
            ArithmeticExpr::Sub(a, b) => write!(f, "({a} - {b})"),
            ArithmeticExpr::Mul(v) => write!(f, "({})", join(v, " · ")),
            ArithmeticExpr::Div(a, b) => write!(f, "({a} / {b})"),
            ArithmeticExpr::Neg(a) => write!(f, "(-{a})"),
            ArithmeticExpr::Pow(a, b) => write!(f, "({a}^{b})"),
            ArithmeticExpr::Function { name, arguments } => {
                write!(f, "{name}({})", join(arguments, ", "))
            }
            ArithmeticExpr::Lag(a, dim) => write!(f, "lag[{dim}]({a})"),
            ArithmeticExpr::Index(a, dim) => write!(f, "{a}[{dim}]"),
        }
    }
}

fn join(v: &[ArithmeticExpr], sep: &str) -> String {
    v.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(sep)
}
