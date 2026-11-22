//! An enumeration a message carries as one byte: each value has its code,
//! and a byte that is no value's code is refused by name.

/// Declares an enumeration carried as one byte, with every value listed
/// in `ALL`, `code` for writing it and `of` for reading it.
macro_rules! coded {
    ($(#[$meta:meta])* $name:ident, $what:literal { $($(#[$variant_meta:meta])* $variant:ident = $code:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
        }

        impl $name {
            /// Every value.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// The byte a message carries for the value.
            #[must_use]
            pub const fn code(self) -> u8 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// The value of `code`.
            ///
            /// # Errors
            /// [`ProtocolError::BadValue`](crate::ProtocolError::BadValue)
            /// for a byte that is no value's code.
            pub const fn of(code: u8) -> Result<Self, $crate::ProtocolError> {
                match code {
                    $($code => Ok(Self::$variant),)+
                    _ => Err($crate::ProtocolError::BadValue($what)),
                }
            }
        }
    };
}

pub(crate) use coded;
