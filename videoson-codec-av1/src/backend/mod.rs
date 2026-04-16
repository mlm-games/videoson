#[cfg(all(feature = "backend-rav1d", feature = "backend-dav1d"))]
compile_error!("Enable only one AV1 backend: backend-rav1d OR backend-dav1d");

#[cfg(feature = "backend-rav1d")]
mod rav1d_backend;
#[cfg(feature = "backend-rav1d")]
pub use rav1d_backend::Av1Decoder;

#[cfg(all(
    feature = "backend-dav1d",
    not(any(target_arch = "wasm32", target_os = "android"))
))]
mod dav1d_backend;
#[cfg(all(
    feature = "backend-dav1d",
    not(any(target_arch = "wasm32", target_os = "android"))
))]
pub use dav1d_backend::Av1Decoder;

#[cfg(all(
    feature = "backend-dav1d",
    any(target_arch = "wasm32", target_os = "android")
))]
compile_error!("backend-dav1d is not supported on wasm32 or Android (temporary restriction).");

#[cfg(not(any(feature = "backend-rav1d", feature = "backend-dav1d")))]
compile_error!(
    "Enable an AV1 backend feature: backend-rav1d (pure Rust) or backend-dav1d (temporary)."
);
