//! 和数据库里 TEXT + CHECK 列对应的枚举。

/// 字符串不是这个枚举的合法取值。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("「{value}」不是合法的 {type_name}，可选：{expected}")]
pub struct UnknownValue {
    pub type_name: &'static str,
    pub value: String,
    pub expected: String,
}

/// 定义一个取值是 snake_case 字符串的枚举。
///
/// 生成：serde（序列化成同样的字符串）、`utoipa::ToSchema`、`as_str`、`ALL`、
/// `FromStr`、`Display`；开了 `sqlx` feature 时还能直接读写 PostgreSQL 的 TEXT 列。
///
/// 取值那一格写成 `tt` 而不是 `literal`：`literal` 传给派生宏时外面多一层看不见的括号，
/// serde 认得，utoipa 认不出，接口文档里的取值就会变成变体名。
macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $text:tt ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash,
            serde::Serialize, serde::Deserialize, utoipa::ToSchema,
        )]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $text)] $variant ),+
        }

        impl $name {
            /// 全部取值，按定义的顺序。
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];

            /// 数据库和 JSON 里的写法。
            pub const fn as_str(self) -> &'static str {
                match self { $( $name::$variant => $text ),+ }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::text_enum::UnknownValue;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok($name::$variant), )+
                    other => Err($crate::text_enum::UnknownValue {
                        type_name: stringify!($name),
                        value: other.to_owned(),
                        expected: [ $( $text ),+ ].join("、"),
                    }),
                }
            }
        }

        #[cfg(feature = "sqlx")]
        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <&str as sqlx::Type<sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <&str as sqlx::Type<sqlx::Postgres>>::compatible(ty)
            }
        }

        #[cfg(feature = "sqlx")]
        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let text = <&str as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
                Ok(text.parse::<$name>()?)
            }
        }

        #[cfg(feature = "sqlx")]
        impl<'q> sqlx::Encode<'q, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<'q, sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
            }
        }
    };
}

pub(crate) use text_enum;
