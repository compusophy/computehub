//! Host-capability tables as DATA, declared once. A purpose-sized language's
//! host surface otherwise ends up declared several times — the parents'
//! rustlite kept the same table in `typecheck.rs` (typed signatures), in
//! `loader.rs` (wasm import objects), and in a hand-synced JS worker, with
//! "ABI mirrors typecheck.rs" comments as the only contract; it drifted and
//! bit repeatedly. Here ONE [`CapTable`] drives all of it:
//!
//! - **checking** — [`Cap::check_args`] compares declared params to a call
//!   site's argument types,
//! - **import emission** — iterate the table in declared order; the index IS
//!   the import order,
//! - **human docs** — [`CapTable::docs_markdown`],
//! - **cross-boundary parity** — [`CapTable::manifest`] is a canonical text
//!   form and [`CapTable::manifest_hash`] its FNV-1a-64. The far side of a
//!   boundary (a JS worker, another process) recomputes the hash from ITS
//!   copy of the table and a CI test compares the two — drift becomes a red
//!   build instead of a runtime mystery.
//!
//! The type vocabulary is the language's, supplied via [`Ty`] (a small total
//! language may say `i64`/`bool`; a wasm language says
//! `i32`/`i64`/`f32`/`f64`). The manifest format and [`Ty::sym`] strings are
//! ABI: changing either changes every hash on purpose.
//!
//! The capability table is the COMPLETE effect bound of whatever runs behind
//! it: a program provably cannot touch anything the table does not name.
//!
//! Zero dependencies. Native + wasm32.
//!
//! Forked from litelite's caplite 0.2.0 (commit `4f5e056`).

/// A language's type vocabulary, as stable lowercase symbols. `sym` strings
/// appear in signatures and the parity manifest — changing one is an ABI
/// change and moves every manifest hash. The contract [`CapTable::validate`]
/// enforces: every sym must be a plain identifier (a `,`, `(`, or newline in
/// a sym could forge or collapse manifest lines), and distinct variants must
/// return distinct syms or the manifest cannot tell them apart.
pub trait Ty: Copy + PartialEq {
    fn sym(&self) -> &'static str;
}

/// One host capability: a named, typed, fuel-costed import. Fields are
/// `'static` so tables are plain `static` data a language declares once.
#[derive(Debug, Clone, Copy)]
pub struct Cap<T: 'static> {
    /// Namespace (wasm import module / doc grouping).
    pub module: &'static str,
    /// Function name within the module.
    pub name: &'static str,
    /// Parameter types, in order.
    pub params: &'static [T],
    /// Result type; `None` renders as `()` (no value).
    pub result: Option<T>,
    /// Fuel surcharge one call costs, beyond the caller's base burn. Part of
    /// the manifest: a cost change changes observable behavior.
    pub cost: u64,
    /// One-line human description (docs only, NOT in the manifest).
    pub doc: &'static str,
}

impl<T: Ty> Cap<T> {
    /// Compact signature: `name(i64,bool)->i64` (result `()` when absent).
    /// This exact rendering appears in the manifest — it is ABI.
    pub fn sig(&self) -> String {
        let params: Vec<&str> = self.params.iter().map(|t| t.sym()).collect();
        let result = self.result.map_or("()", |t| t.sym());
        format!("{}({})->{}", self.name, params.join(","), result)
    }

    /// Check a call site's argument types against the declared params.
    pub fn check_args(&self, got: &[T]) -> Result<(), ArgError<T>> {
        check_args(self.params, got)
    }
}

/// [`Cap::check_args`] as a free function, for callers that copied the
/// `'static` param slice out of a table borrow.
pub fn check_args<T: Ty>(params: &[T], got: &[T]) -> Result<(), ArgError<T>> {
    if got.len() != params.len() {
        return Err(ArgError::Arity {
            expected: params.len(),
            got: got.len(),
        });
    }
    for (index, (want, have)) in params.iter().zip(got).enumerate() {
        if want != have {
            return Err(ArgError::Type {
                index,
                expected: *want,
                got: *have,
            });
        }
    }
    Ok(())
}

/// Why a call site does not match a capability's declared params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgError<T> {
    Arity {
        expected: usize,
        got: usize,
    },
    /// `index` is 0-based; `Display` renders it 1-based for humans.
    Type {
        index: usize,
        expected: T,
        got: T,
    },
}

impl<T: Ty> std::fmt::Display for ArgError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArgError::Arity { expected, got } => {
                write!(f, "expects {expected} argument(s), got {got}")
            }
            ArgError::Type {
                index,
                expected,
                got,
            } => {
                write!(
                    f,
                    "argument {} must be {}, got {}",
                    index + 1,
                    expected.sym(),
                    got.sym()
                )
            }
        }
    }
}

// `Error: Debug`, and `Ty` does not require it — so the bound is `Ty + Debug`
// rather than widening `Ty` itself, which would constrain every implementor.
impl<T: Ty + std::fmt::Debug> std::error::Error for ArgError<T> {}

/// A capability table: the single declaration everything else derives from.
/// Declared order is import order; the index of a cap is stable ABI.
#[derive(Debug, Clone, Copy)]
pub struct CapTable<T: 'static> {
    caps: &'static [Cap<T>],
}

impl<T: Ty> CapTable<T> {
    /// Wrap a static table. Run [`validate`](Self::validate) in a test (or at
    /// host startup) — `new` stays `const` so tables can be `static`.
    pub const fn new(caps: &'static [Cap<T>]) -> Self {
        Self { caps }
    }

    /// Reject duplicate `(module, name)` declarations, and any module, name,
    /// or type symbol that is not a plain identifier
    /// (`[A-Za-z_][A-Za-z0-9_]*`) — a stray `,`, `(`, or newline in ANY of
    /// the strings the manifest interpolates could forge or collapse manifest
    /// lines, the exact contract this crate exists to protect. (Type symbols
    /// are as much a channel as names: a sym `"i64,i64"` on a one-param cap
    /// renders byte-identically to two `"i64"` params.)
    pub fn validate(&self) -> Result<(), String> {
        for (i, cap) in self.iter() {
            for part in [cap.module, cap.name] {
                if !is_ident(part) {
                    return Err(format!(
                        "capability `{}.{}`: `{part}` is not an identifier",
                        cap.module, cap.name
                    ));
                }
            }
            for t in cap.params.iter().copied().chain(cap.result) {
                if !is_ident(t.sym()) {
                    return Err(format!(
                        "capability `{}.{}`: type symbol `{}` is not an identifier",
                        cap.module,
                        cap.name,
                        t.sym()
                    ));
                }
            }
            for (j, other) in self.iter() {
                if j > i && other.module == cap.module && other.name == cap.name {
                    return Err(format!(
                        "duplicate capability `{}.{}`",
                        cap.module, cap.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// [`validate`](Self::validate), plus: bare names must be unique ACROSS
    /// modules. For languages whose call sites have a single flat namespace
    /// (bare `name(args)` calls), [`find`](Self::find) is only sound
    /// under this rule — enforce it here once instead of in every language.
    pub fn validate_flat(&self) -> Result<(), String> {
        self.validate()?;
        for (i, cap) in self.iter() {
            for (j, other) in self.iter() {
                if j > i && other.name == cap.name {
                    return Err(format!(
                        "capability name `{}` appears in both `{}` and `{}`; flat call sites use bare names",
                        cap.name, cap.module, other.module
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.caps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }

    /// Caps with their stable indices, in declared (= import) order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Cap<T>)> {
        self.caps.iter().enumerate()
    }

    /// Look up by full `(module, name)`.
    pub fn get(&self, module: &str, name: &str) -> Option<(usize, &Cap<T>)> {
        self.iter()
            .find(|(_, c)| c.module == module && c.name == name)
    }

    /// Look up by bare name (first match) — for languages whose call sites
    /// have a single flat namespace. Such languages should also enforce that
    /// bare names are unique across modules.
    pub fn find(&self, name: &str) -> Option<(usize, &Cap<T>)> {
        self.iter().find(|(_, c)| c.name == name)
    }

    /// The canonical manifest — the parity contract for the far side of a
    /// boundary. STABLE FORMAT (version-headed; bump the version to change
    /// it): one header line, then one line per cap in declared order:
    ///
    /// ```text
    /// caplite-manifest/1
    /// 0 counter.next()->i64 cost=1
    /// 1 math.mix(i64,i64)->i64 cost=2
    /// ```
    ///
    /// The `caplite-manifest/1` header predates the fork and is kept
    /// verbatim: it is part of the hashed bytes.
    pub fn manifest(&self) -> String {
        let mut out = String::from("caplite-manifest/1\n");
        for (i, cap) in self.iter() {
            out.push_str(&format!(
                "{i} {}.{} cost={}\n",
                cap.module,
                cap.sig(),
                cap.cost
            ));
        }
        out
    }

    /// FNV-1a-64 of [`manifest`](Self::manifest) bytes. The far side
    /// recomputes this from its own copy of the table (FNV-1a-64 is a
    /// ten-line function in any language; in JS use BigInt) and a CI test
    /// asserts equality — table drift becomes a red build.
    pub fn manifest_hash(&self) -> u64 {
        fnv1a_64(self.manifest().as_bytes())
    }

    /// A markdown table for human documentation. Doc strings are sanitized
    /// (`|` escaped, newlines flattened) so a description can never break or
    /// forge table rows.
    pub fn docs_markdown(&self) -> String {
        let mut out = String::from(
            "| # | capability | signature | cost | description |\n|---|---|---|---|---|\n",
        );
        for (i, cap) in self.iter() {
            let doc = cap.doc.replace(['\n', '\r'], " ").replace('|', "\\|");
            out.push_str(&format!(
                "| {i} | `{}.{}` | `{}` | {} | {doc} |\n",
                cap.module,
                cap.name,
                cap.sig(),
                cap.cost
            ));
        }
        out
    }
}

fn is_ident(s: &str) -> bool {
    let mut bytes = s.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// FNV-1a, 64-bit. Public so consumers can hash manifests received over a
/// boundary without re-deriving the constants.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum T {
        I64,
        Bool,
    }
    impl Ty for T {
        fn sym(&self) -> &'static str {
            match self {
                T::I64 => "i64",
                T::Bool => "bool",
            }
        }
    }

    const CAPS: &[Cap<T>] = &[
        Cap {
            module: "counter",
            name: "next",
            params: &[],
            result: Some(T::I64),
            cost: 1,
            doc: "Monotonic counter.",
        },
        Cap {
            module: "math",
            name: "mix",
            params: &[T::I64, T::I64],
            result: Some(T::I64),
            cost: 2,
            doc: "XOR of two integers.",
        },
        Cap {
            module: "log",
            name: "emit",
            params: &[T::Bool],
            result: None,
            cost: 0,
            doc: "Fire-and-forget.",
        },
    ];
    static TABLE: CapTable<T> = CapTable::new(CAPS);

    #[test]
    fn lookup_by_full_and_bare_name() {
        assert_eq!(TABLE.get("math", "mix").unwrap().0, 1);
        assert_eq!(TABLE.find("mix").unwrap().0, 1);
        assert!(TABLE.get("math", "next").is_none());
        assert!(TABLE.find("nope").is_none());
        assert_eq!(TABLE.len(), 3);
        assert!(!TABLE.is_empty());
        assert!(TABLE.validate().is_ok());
    }

    #[test]
    fn duplicates_fail_validation() {
        static DUP: &[Cap<T>] = &[
            Cap {
                module: "a",
                name: "x",
                params: &[],
                result: None,
                cost: 0,
                doc: "",
            },
            Cap {
                module: "a",
                name: "x",
                params: &[T::I64],
                result: None,
                cost: 0,
                doc: "",
            },
        ];
        let err = CapTable::new(DUP).validate().unwrap_err();
        assert!(err.contains("`a.x`"), "{err}");
    }

    #[test]
    fn non_ident_type_symbols_fail_validation() {
        // A comma-bearing sym collapses arity (one "i64,i64" param renders
        // like two "i64" params); a newline sym forges lines. Reject the class.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct Evil(&'static str);
        impl Ty for Evil {
            fn sym(&self) -> &'static str {
                self.0
            }
        }
        for bad in ["i64,i64", "", "()", ")->x cost=0\n1 evil.own()->(", "a b"] {
            let caps: &'static [Cap<Evil>] = Box::leak(Box::new([Cap {
                module: "m",
                name: "f",
                params: Box::leak(Box::new([Evil(bad)])),
                result: None,
                cost: 0,
                doc: "",
            }]));
            let err = CapTable::new(caps).validate().unwrap_err();
            assert!(err.contains("type symbol"), "`{bad}`: {err}");
        }
        assert!(TABLE.validate().is_ok()); // honest syms still pass
    }

    #[test]
    fn validate_flat_rejects_cross_module_bare_name_dups() {
        assert!(TABLE.validate_flat().is_ok());
        static AMBIG: &[Cap<T>] = &[
            Cap {
                module: "a",
                name: "x",
                params: &[],
                result: Some(T::I64),
                cost: 0,
                doc: "",
            },
            Cap {
                module: "b",
                name: "x",
                params: &[],
                result: Some(T::I64),
                cost: 0,
                doc: "",
            },
        ];
        let err = CapTable::new(AMBIG).validate_flat().unwrap_err();
        assert!(err.contains("`x`"), "{err}");
    }

    #[test]
    fn docs_markdown_sanitizes_doc_strings() {
        static SNEAKY: &[Cap<T>] = &[Cap {
            module: "m",
            name: "f",
            params: &[],
            result: Some(T::I64),
            cost: 0,
            doc: "pipe | here\n| forged | row | injected | here |",
        }];
        let docs = CapTable::new(SNEAKY).docs_markdown();
        assert_eq!(docs.lines().count(), 3); // header + divider + ONE row
        assert!(docs.contains("pipe \\| here"), "{docs}");
    }

    #[test]
    fn non_ident_names_fail_validation() {
        // A newline in a name could forge manifest lines; reject the class.
        for bad in ["a\nb", "", "1x", "a.b", "a b", "café"] {
            let caps: &'static [Cap<T>] = Box::leak(Box::new([Cap {
                module: "ok",
                name: Box::leak(bad.to_string().into_boxed_str()),
                params: &[],
                result: None,
                cost: 0,
                doc: "",
            }]));
            assert!(
                CapTable::new(caps).validate().is_err(),
                "`{bad}` should fail"
            );
        }
    }

    #[test]
    fn check_args_catches_arity_and_type() {
        let (_, mix) = TABLE.find("mix").unwrap();
        assert!(mix.check_args(&[T::I64, T::I64]).is_ok());
        let e = mix.check_args(&[T::I64]).unwrap_err();
        assert_eq!(
            e,
            ArgError::Arity {
                expected: 2,
                got: 1
            }
        );
        assert_eq!(e.to_string(), "expects 2 argument(s), got 1");
        let e = mix.check_args(&[T::I64, T::Bool]).unwrap_err();
        assert_eq!(e.to_string(), "argument 2 must be i64, got bool");
    }

    #[test]
    fn manifest_is_the_exact_stable_contract() {
        assert_eq!(
            TABLE.manifest(),
            "caplite-manifest/1\n\
             0 counter.next()->i64 cost=1\n\
             1 math.mix(i64,i64)->i64 cost=2\n\
             2 log.emit(bool)->() cost=0\n"
        );
        assert_eq!(TABLE.manifest_hash(), fnv1a_64(TABLE.manifest().as_bytes()));
        // Any ABI change — name, order, type symbol, cost — moves the hash.
        static REORDERED: &[Cap<T>] = &[CAPS[1], CAPS[0], CAPS[2]];
        assert_ne!(
            CapTable::new(REORDERED).manifest_hash(),
            TABLE.manifest_hash()
        );
    }

    #[test]
    fn fnv1a_64_matches_the_published_vectors() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn docs_render_as_a_markdown_table() {
        let docs = TABLE.docs_markdown();
        assert!(
            docs.contains("| 1 | `math.mix` | `mix(i64,i64)->i64` | 2 | XOR of two integers. |")
        );
        assert_eq!(docs.lines().count(), 2 + CAPS.len());
    }
}
