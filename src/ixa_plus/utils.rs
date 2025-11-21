#[macro_export]
macro_rules! format_iter {
    ($vec:expr, |$map_args:tt| $map_body:expr, sep = $sep:expr) => {
        $vec.iter()
            .map(|$map_args| format!($map_body))
            .collect::<Vec<_>>()
            .join($sep)
    };
    ($vec:expr, |$map_args:tt| $map_body:expr) => {
        format_iter!($vec, |$map_args| $map_body, sep = ", ")
    };
}
