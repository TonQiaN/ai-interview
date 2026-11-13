//! 迁移文件是编译时嵌进来的（`sqlx::migrate!`）。告诉 cargo：迁移目录变了要重新编译，
//! 不然加了新迁移之后，本地测试用的还是旧的表结构。

fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
