use crate::*;

/// Enable high-precision decimal arithmetic for math commands.
///
/// When enabled, math commands will use rust_decimal::Decimal for calculations
/// instead of f64, providing arbitrary precision and avoiding floating-point
/// rounding errors. This is particularly useful for financial calculations
/// and other scenarios where precision is critical.
///
/// Note: Decimal arithmetic is slower than f64 arithmetic, so this should
/// only be enabled when precision is more important than performance.
pub static DECIMAL_MATH: ExperimentalOption = ExperimentalOption::new(&DecimalMath);

struct DecimalMath;

impl ExperimentalOptionMarker for DecimalMath {
    const IDENTIFIER: &'static str = "decimal-math";
    const DESCRIPTION: &'static str = "Use high-precision decimal arithmetic for math commands";
    const STATUS: Status = Status::OptIn; // Disabled by default
    const SINCE: Version = (0, 116, 0);
    // TODO: file a tracking issue on GitHub for this experimental option
    const ISSUE: u32 = 0;
}
