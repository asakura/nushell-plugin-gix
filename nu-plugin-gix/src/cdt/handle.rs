//! The contract shared by every custom value this plugin hands to Nushell.

use nu_protocol::{CustomValue, Value};

/// A stateful handle: a custom value that commands accept as pipeline input by type.
pub trait Handle: CustomValue + Sized + 'static {
    /// The name Nushell shows for this type, e.g. in `describe` and in type errors.
    const TYPE_NAME: &'static str;

    /// How a script obtains this handle, for the hint in wrong-input errors.
    const OBTAINED_BY: &'static str;
}

/// Borrows `value` as the handle type `H`, or `None` when it is anything else.
#[must_use = "returns the borrowed handle; the value is unchanged"]
pub fn downcast<H>(value: &Value) -> Option<&H>
where
    H: Handle,
{
    let custom = value.as_custom_value().ok()?;

    custom.as_any().downcast_ref::<H>()
}
