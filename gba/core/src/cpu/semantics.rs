//! Pure CPU semantic helpers owned by the unified micro-op engine,
//! split by area: the barrel shifter group and the multiply group.

pub(crate) mod multiply;
pub(crate) mod shifter;

#[cfg(test)]
mod tests;
