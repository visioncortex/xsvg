//! Native host for loadable geometry generators (docs/Extensibility.md).
//!
//! The browser host is JS driving `WebAssembly`; this is its native counterpart,
//! running the **same module bytes** through [`wasmi`] — a pure-Rust interpreter,
//! so the CLI stays a single self-contained binary with no JIT and no C toolchain.
//! Identical bytes on both hosts is what makes native/browser output identical by
//! construction, rather than by carefully matching two implementations.
//!
//! Two things this host can do that the browser one cannot: **bound a runaway
//! generator** (wasmi fuel — a synchronous call on the browser's main thread is
//! not interruptible), and instantiate **per call**, so a module cannot carry
//! state even between two elements of one document.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use xsvg_core::{
    ChildMode, ElementInfo, GenRequest, GenResult, GeneratorHost, Import, InputMode,
};
use xsvg_plugin::{host, json, Json};

/// The ABI this host speaks (docs/Extensibility.md §4).
const ABI_VERSION: i32 = 1;

/// Instructions a single `xsvg_generate` call may execute before it is cut off.
/// Generous next to any real generator (the clipart arrow costs a few hundred
/// thousand), and small enough that a non-terminating module fails in well under
/// a second instead of hanging the compile.
const FUEL: u64 = 500_000_000;

/// A module that loaded and validated, plus what its manifest declared.
struct Loaded {
    module: wasmi::Module,
    /// element name → (input mode, child mode)
    elements: HashMap<String, (InputMode, ChildMode)>,
}

/// The generator modules a document's `<x:import>`s resolved to.
///
/// A namespace that failed to load is kept as its failure reason: every element
/// in it then degrades with that reason rather than a generic "not bound", so the
/// output names the thing to fix.
#[derive(Default)]
pub struct DiskGenerators {
    loaded: HashMap<String, Loaded>,
    failed: HashMap<String, String>,
}

impl DiskGenerators {
    /// Resolve every import against the document's path and load it. Failures are
    /// recorded, never fatal — a document with a broken import still compiles.
    pub fn load(imports: &[Import], base: &str) -> Self {
        let mut out = Self::default();
        let dir = Path::new(base)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        for im in imports {
            match load_one(&dir, im) {
                Ok(l) => {
                    out.loaded.insert(im.ns.clone(), l);
                }
                Err(e) => {
                    out.failed.insert(im.ns.clone(), e);
                }
            }
        }
        out
    }

}

/// Whether `src` names a URI scheme rather than a path (docs/Sandbox.md §2).
///
/// Tests the scheme, not `://`: `data:` and `blob:` carry no authority and would
/// otherwise fall through to a filesystem read — harmless here, since there is
/// nothing to fetch with, but it reports "no such file" for something that was
/// never a file. Two characters minimum, so a Windows drive letter stays a path.
fn has_scheme(src: &str) -> bool {
    let Some(colon) = src.find(':') else {
        return false;
    };
    let scheme = &src[..colon];
    colon >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn load_one(dir: &Path, im: &Import) -> Result<Loaded, String> {
    // A module is a local file like any other dependency; a URL is the browser's
    // business, and a CLI silently reaching the network would be a surprise.
    let path = dir.join(&im.src);
    if has_scheme(&im.src) {
        return Err(format!("cannot fetch {} — give a local path", im.src));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(want) = &im.integrity {
        verify_integrity(&bytes, want)?;
    }

    let engine = engine();
    let module = wasmi::Module::new(&engine, &bytes[..]).map_err(|e| format!("invalid wasm: {e}"))?;

    // One throwaway instantiation to read the manifest and check the ABI.
    let mut store = new_store(&engine);
    let instance = instantiate(&mut store, &module)?;
    let abi = call_i32(&mut store, &instance, "xsvg_abi_version")?;
    if abi != ABI_VERSION {
        return Err(format!(
            "module speaks ABI {abi}, host speaks {ABI_VERSION}"
        ));
    }
    let ptr = call_i32(&mut store, &instance, "xsvg_describe")?;
    let text = read_reply(&mut store, &instance, ptr)?;
    let manifest = json::parse(&text).ok_or("module manifest is not valid JSON")?;

    // The module must claim the namespace it was bound to, or a document could be
    // pointed at a module answering for someone else's elements.
    let declared = manifest.get("ns").and_then(Json::as_str).unwrap_or("");
    if declared != im.ns {
        return Err(format!(
            "module declares namespace {}, bound as {}",
            if declared.is_empty() { "(none)" } else { declared },
            im.ns
        ));
    }

    let mut elements = HashMap::new();
    if let Some(Json::Obj(kv)) = manifest.get("elements") {
        for (name, spec) in kv {
            let input = match spec.get("input").and_then(Json::as_str) {
                Some("required") => InputMode::Required,
                Some("optional") => InputMode::Optional,
                _ => InputMode::None,
            };
            let children = match spec.get("children").and_then(Json::as_str) {
                Some("container") => ChildMode::Container,
                _ => ChildMode::None,
            };
            elements.insert(name.clone(), (input, children));
        }
    }
    Ok(Loaded { module, elements })
}

/// Check an SRI-style `sha256-<base64>` digest.
fn verify_integrity(bytes: &[u8], want: &str) -> Result<(), String> {
    let Some(b64) = want.strip_prefix("sha256-") else {
        return Err(format!("unsupported integrity algorithm in {want:?}"));
    };
    use sha2::Digest;
    let got = base64(&sha2::Sha256::digest(bytes));
    if got == b64 {
        Ok(())
    } else {
        Err("integrity mismatch".to_string())
    }
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i)) as usize & 0x3F] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// ---- wasmi plumbing -------------------------------------------------------

fn engine() -> wasmi::Engine {
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    wasmi::Engine::new(&config)
}

fn new_store(engine: &wasmi::Engine) -> wasmi::Store<()> {
    let mut store = wasmi::Store::new(engine, ());
    store.set_fuel(FUEL).ok();
    store
}

/// Instantiate with **no imports at all** — a generator gets no WASI, no host
/// functions, and no way to observe anything but its own request.
fn instantiate(
    store: &mut wasmi::Store<()>,
    module: &wasmi::Module,
) -> Result<wasmi::Instance, String> {
    wasmi::Linker::<()>::new(module.engine())
        .instantiate_and_start(&mut *store, module)
        .map_err(|e| format!("instantiate: {e}"))
}

fn call_i32(
    store: &mut wasmi::Store<()>,
    instance: &wasmi::Instance,
    name: &str,
) -> Result<i32, String> {
    instance
        .get_typed_func::<(), i32>(&*store, name)
        .map_err(|_| format!("module exports no {name}"))?
        .call(store, ())
        .map_err(|e| format!("{name}: {e}"))
}

fn memory(store: &wasmi::Store<()>, instance: &wasmi::Instance) -> Result<wasmi::Memory, String> {
    instance
        .get_memory(store, "memory")
        .ok_or_else(|| "module exports no memory".to_string())
}

/// Read a `u32`-LE length-prefixed UTF-8 reply and hand the buffer back.
fn read_reply(
    store: &mut wasmi::Store<()>,
    instance: &wasmi::Instance,
    ptr: i32,
) -> Result<String, String> {
    let mem = memory(store, instance)?;
    let at = usize::try_from(ptr).map_err(|_| "bad reply pointer")?;
    let mut len_bytes = [0u8; 4];
    mem.read(&*store, at, &mut len_bytes)
        .map_err(|e| format!("reply header: {e}"))?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    mem.read(&*store, at + 4, &mut buf)
        .map_err(|e| format!("reply body: {e}"))?;
    let text = String::from_utf8(buf).map_err(|_| "reply is not valid UTF-8".to_string())?;
    instance
        .get_typed_func::<(i32, i32), ()>(&*store, "xsvg_free")
        .map_err(|_| "module exports no xsvg_free".to_string())?
        .call(store, (ptr, (len + 4) as i32))
        .map_err(|e| format!("xsvg_free: {e}"))?;
    Ok(text)
}

/// Run one generate call in a **fresh instance**, so nothing a module retains can
/// be observed by the next element, and a trap or exhausted fuel costs only the
/// element that caused it.
fn run(loaded: &Loaded, payload: &str) -> Result<String, String> {
    let engine = loaded.module.engine().clone();
    let mut store = new_store(&engine);
    let instance = instantiate(&mut store, &loaded.module)?;

    let bytes = payload.as_bytes();
    let ptr = instance
        .get_typed_func::<i32, i32>(&store, "xsvg_alloc")
        .map_err(|_| "module exports no xsvg_alloc".to_string())?
        .call(&mut store, bytes.len() as i32)
        .map_err(|e| format!("xsvg_alloc: {e}"))?;
    // read the memory handle after allocating: growth may have replaced it
    memory(&store, &instance)?
        .write(&mut store, ptr as usize, bytes)
        .map_err(|e| format!("writing request: {e}"))?;

    let reply = instance
        .get_typed_func::<(i32, i32), i32>(&store, "xsvg_generate")
        .map_err(|_| "module exports no xsvg_generate".to_string())?
        .call(&mut store, (ptr, bytes.len() as i32))
        .map_err(|e| {
            if store.get_fuel().is_ok_and(|f| f == 0) {
                "generator ran out of fuel".to_string()
            } else {
                format!("generator trapped: {e}")
            }
        })?;
    read_reply(&mut store, &instance, reply)
}

impl GeneratorHost for DiskGenerators {
    fn element(&self, ns: &str, tag: &str) -> ElementInfo {
        let Some(l) = self.loaded.get(ns) else {
            // A namespace whose module failed claims its elements anyway, so
            // `generate` can decline each with the real reason.
            return if self.failed.contains_key(ns) {
                ElementInfo::Ready {
                    input: InputMode::None,
                    children: ChildMode::None,
                }
            } else {
                ElementInfo::Unbound
            };
        };
        match l.elements.get(tag) {
            Some(&(input, children)) => ElementInfo::Ready { input, children },
            None => ElementInfo::UnknownElement,
        }
    }

    fn generate(&self, ns: &str, tag: &str, req: &GenRequest) -> GenResult {
        let Some(l) = self.loaded.get(ns) else {
            return GenResult::Error(
                self.failed
                    .get(ns)
                    .cloned()
                    .unwrap_or_else(|| "module not loaded".to_string()),
            );
        };
        let kids: Vec<host::ChildIn> = req
            .children
            .iter()
            .map(|c| (c.svg.as_str(), c.bbox))
            .collect();
        let payload = host::encode_request(
            tag,
            &req.attrs,
            req.input_d.as_deref(),
            &kids,
            req.tolerance,
        );
        match run(l, &payload).and_then(|r| host::decode_reply(&r)) {
            Ok(svg) => GenResult::Svg(svg),
            Err(why) => GenResult::Error(why),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        // the integrity check is only as trustworthy as this
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // The bytes SRI actually carries: a sha256 digest, padded — this is the
        // exact spelling Node's digest("base64") and the browser's btoa() produce,
        // which is what lets one `integrity` attribute satisfy all three hosts.
        use sha2::Digest;
        assert_eq!(
            base64(&sha2::Sha256::digest(b"abc")),
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        );
    }

    #[test]
    fn integrity_accepts_the_right_hash_and_rejects_everything_else() {
        use sha2::Digest;
        let bytes = b"some module bytes";
        let good = format!("sha256-{}", base64(&sha2::Sha256::digest(bytes)));
        assert!(verify_integrity(bytes, &good).is_ok());
        assert!(verify_integrity(b"other bytes", &good).is_err());
        assert!(verify_integrity(bytes, "sha256-AAAA").is_err());
        assert!(verify_integrity(bytes, "md5-whatever").is_err());
    }
}

#[cfg(test)]
mod sandbox_tests {
    use super::*;

    fn import(src: &str) -> Import {
        Import {
            ns: "https://x.example/c".into(),
            src: src.into(),
            integrity: None,
        }
    }

    #[test]
    fn a_module_is_never_fetched_over_the_network() {
        // docs/Sandbox.md §2: the native hosts take local paths only. A CLI or a
        // build step reaching out on its own would be a surprise, and it is also
        // the one host with no origin to reason about — so the rule is absolute
        // rather than conditional on a hash.
        let dir = Path::new(".");
        for src in [
            "https://cdn.example/m.wasm",
            "http://cdn.example/m.wasm",
            "file:///tmp/m.wasm",
            "data:application/wasm;base64,AGFzbQ==",
        ] {
            let err = load_one(dir, &import(src)).err().expect("remote src must be refused");
            assert!(
                err.contains("give a local path"),
                "`{src}` was not refused as remote: {err}"
            );
        }
        // a plain relative path is not refused for *that* reason — it gets as far
        // as the filesystem, which is the point
        let err = load_one(dir, &import("nope.wasm")).err().expect("missing file must error");
        assert!(!err.contains("give a local path"), "{err}");
    }
}
