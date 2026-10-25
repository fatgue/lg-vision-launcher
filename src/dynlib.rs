//! Runtime-loaded C APIs. Nothing is linked at build time, so the ARM binary cross-compiles
//! without TV sysroots and the desktop preview only needs SDL2.dll next to the executable.

/// Declares a struct of function pointers resolved from the first shared library that opens.
macro_rules! dynamic_api {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( fn $field:ident = $sym:literal : fn($($arg:ty),*) $(-> $ret:ty)?; )*
        }
    ) => {
        $(#[$meta])*
        pub struct $name {
            _lib: libloading::Library,
            $( pub $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?, )*
        }

        impl $name {
            pub fn open(candidates: &[std::path::PathBuf]) -> Result<Self, String> {
                let mut errors = Vec::new();
                for path in candidates {
                    // SAFETY: loading a trusted system/bundled library; constructors are benign.
                    let lib = match unsafe { libloading::Library::new(path) } {
                        Ok(lib) => lib,
                        Err(error) => {
                            errors.push(format!("{}: {error}", path.display()));
                            continue;
                        }
                    };
                    // SAFETY: each symbol is declared with its C prototype.
                    unsafe {
                        $(
                            let $field = *lib
                                .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!($sym, "\0").as_bytes())
                                .map_err(|error| format!("{}: {error}", $sym))?;
                        )*
                        return Ok(Self { _lib: lib, $( $field, )* });
                    }
                }
                Err(errors.join("; "))
            }
        }
    };
}

/// Declares GL entry points resolved through the context's proc-address loader.
macro_rules! gl_api {
    (
        pub struct $name:ident {
            $( fn $field:ident = $sym:literal : fn($($arg:ty),*) $(-> $ret:ty)?; )*
        }
    ) => {
        pub struct $name {
            $( pub $field: unsafe extern "system" fn($($arg),*) $(-> $ret)?, )*
        }

        impl $name {
            pub fn load(mut get: impl FnMut(&str) -> *const std::ffi::c_void) -> Result<Self, String> {
                Ok(Self {
                    $(
                        $field: {
                            let pointer = get(concat!($sym, "\0"));
                            if pointer.is_null() {
                                return Err(format!("missing GL function {}", $sym));
                            }
                            // SAFETY: the loader returned this entry point for the current context.
                            unsafe {
                                std::mem::transmute::<*const std::ffi::c_void, unsafe extern "system" fn($($arg),*) $(-> $ret)?>(pointer)
                            }
                        },
                    )*
                })
            }
        }
    };
}

pub(crate) use dynamic_api;
pub(crate) use gl_api;
