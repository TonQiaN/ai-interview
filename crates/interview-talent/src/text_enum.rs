//! 和数据库里 TEXT + CHECK 列对应的枚举。写法和 `interview-core` 里的一致，
//! 这里放只有岗位和候选人这条线用的那几个。

/// 定义一个取值是 snake_case 字符串的枚举：serde、OpenAPI、`as_str`、`FromStr`，
/// 以及直接读写 PostgreSQL 的 TEXT 列。
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
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok($name::$variant), )+
                    other => Err(format!(
                        "「{other}」不是合法的 {}，可选：{}",
                        stringify!($name),
                        [ $( $text ),+ ].join("、"),
                    )),
                }
            }
        }

        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <&str as sqlx::Type<sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <&str as sqlx::Type<sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let text = <&str as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
                Ok(text.parse::<$name>()?)
            }
        }

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
