/// A C-like enum over the integer a game stores. Values without a variant decode to
/// `Unknown(raw)`, so nothing is lost. Known values serialize as their snake_case name, unknown
/// ones as the raw number.
///
/// ```text
/// raw_enum! {
///     pub enum Color: u8 {
///         Red = 0,
///         Blue = 1,
///     }
/// }
/// ```
macro_rules! raw_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident: $repr:ty {
            $( $(#[$variant_meta:meta])* $variant:ident = $value:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $( $(#[$variant_meta])* $variant, )+
            /// A value without a known meaning, kept as stored.
            #[serde(untagged)]
            Unknown($repr),
        }

        impl $name {
            pub const fn from_raw(raw: $repr) -> Self {
                match raw {
                    $( $value => Self::$variant, )+
                    other => Self::Unknown(other),
                }
            }

            pub const fn raw(self) -> $repr {
                match self {
                    $( Self::$variant => $value, )+
                    Self::Unknown(raw) => raw,
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    raw_enum! {
        pub enum Sample: u8 {
            First = 0,
            SecondOne = 5,
        }
    }

    #[test]
    fn keeps_unknown_values_and_serializes_names() {
        assert_eq!(Sample::from_raw(5), Sample::SecondOne);
        assert_eq!(Sample::from_raw(9), Sample::Unknown(9));
        assert_eq!(Sample::Unknown(9).raw(), 9);
        assert_eq!(
            serde_json::to_string(&Sample::SecondOne).unwrap(),
            r#""second_one""#
        );
        assert_eq!(serde_json::to_string(&Sample::Unknown(9)).unwrap(), "9");
    }
}
