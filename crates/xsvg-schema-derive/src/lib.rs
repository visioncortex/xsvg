//! Derives for [`xsvg-schema`](https://docs.rs/xsvg-schema): turn a struct into
//! both an element description and the code that reads it.
//!
//! The point is that the two cannot be written separately. A parameter with no
//! field cannot be read; a field with no use is a `dead_code` warning. Neither a
//! hand-written JSON manifest nor a table of `attr(…)` calls has that property,
//! and both drift within a release.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Lit, Meta, Type};

/// `#[derive(Params)]` — an element's parameters.
///
/// Struct attributes: `#[element(name = "arrow", doc = "…", input = "required",
/// children = "container")]`. Field attributes: `#[param(name = "…", default = …,
/// min = …, max = …, step = …, values = […], color, doc = "…")]`.
///
/// A field's attribute name defaults to its own with `_` → `-` (`head_width` →
/// `head-width`, `r#in` → `in`), so the common case needs no annotation at all.
#[proc_macro_derive(Params, attributes(element, param))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);
    match expand_params(&ast) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `#[derive(ParamEnum)]` — a closed set of spellings for an enum parameter.
///
/// Variant names lower-case with `_` → `-` (`LowerAlpha` → `lower-alpha`), or use
/// `#[value("…")]` to spell one explicitly.
#[proc_macro_derive(ParamEnum, attributes(value))]
pub fn derive_param_enum(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);
    match expand_param_enum(&ast) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ---- shared helpers -------------------------------------------------------

fn err<T>(span: proc_macro2::Span, msg: &str) -> syn::Result<T> {
    Err(syn::Error::new(span, msg))
}

/// `head_width` → `head-width`; a raw identifier drops its `r#`.
fn attr_name(ident: &Ident) -> String {
    ident.to_string().trim_start_matches("r#").replace('_', "-")
}

fn kebab_variant(ident: &Ident) -> String {
    let s = ident.to_string();
    let mut out = String::with_capacity(s.len() + 2);
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// A literal, or a negated one — `min = -100.0` reaches us as a unary
/// expression rather than a literal, and a range that cannot go negative would
/// be a poor thing to discover at runtime.
fn expr_text(e: &syn::Expr) -> Option<String> {
    match e {
        syn::Expr::Lit(l) => lit_text(&l.lit),
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => {
            expr_text(&u.expr).map(|t| format!("-{t}"))
        }
        _ => None,
    }
}

fn expr_f64(e: &syn::Expr) -> Option<f64> {
    expr_text(e).and_then(|t| t.parse().ok())
}

fn lit_text(lit: &Lit) -> Option<String> {
    match lit {
        Lit::Str(s) => Some(s.value()),
        Lit::Int(i) => Some(i.base10_digits().to_string()),
        Lit::Float(f) => Some(f.base10_digits().to_string()),
        Lit::Bool(b) => Some(b.value.to_string()),
        _ => None,
    }
}

/// The inner type of `Option<T>`, if this is one.
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

fn type_is(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == name))
}

// ---- Params ---------------------------------------------------------------

#[derive(Default)]
struct ElementOpts {
    name: Option<String>,
    doc: Option<String>,
    input: Option<String>,
    children: Option<String>,
    /// Shared parameter blocks this element accepts. Unlike `#[param(flatten)]`
    /// these produce no field: the block is *described* here and read elsewhere,
    /// which is the shape of the font properties every text element takes.
    groups: Vec<proc_macro2::TokenStream>,
}

#[derive(Default)]
struct ParamOpts {
    name: Option<String>,
    default: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    doc: Option<String>,
    /// `superseded_by = "x2, y2"` — parameters whose presence makes this one inert.
    superseded_by: Vec<String>,
    color: bool,
    skip: bool,
    flatten: bool,
}

/// Read `key = value` pairs (and bare flags) out of one `#[attr(…)]`.
fn parse_meta_list(
    meta: &Meta,
    mut on: impl FnMut(&str, Option<&syn::Expr>, proc_macro2::Span) -> syn::Result<()>,
) -> syn::Result<()> {
    let Meta::List(list) = meta else {
        return Ok(());
    };
    let parsed = list.parse_args_with(
        syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
    )?;
    for m in parsed {
        match &m {
            Meta::NameValue(nv) => {
                let key = nv.path.get_ident().map(Ident::to_string).unwrap_or_default();
                on(&key, Some(&nv.value), nv.path.span())?;
            }
            Meta::Path(p) => {
                let key = p.get_ident().map(Ident::to_string).unwrap_or_default();
                on(&key, None, p.span())?;
            }
            Meta::List(l) => {
                let key = l.path.get_ident().map(Ident::to_string).unwrap_or_default();
                on(&key, None, l.path.span())?;
            }
        }
    }
    Ok(())
}

use syn::spanned::Spanned;

fn expand_params(ast: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let Data::Struct(data) = &ast.data else {
        return err(ast.ident.span(), "Params can only be derived for a struct");
    };
    let Fields::Named(fields) = &data.fields else {
        return err(ast.ident.span(), "Params needs named fields");
    };

    let mut el = ElementOpts::default();
    for attr in &ast.attrs {
        if !attr.path().is_ident("element") {
            continue;
        }
        parse_meta_list(&attr.meta, |key, expr, span| {
            if key == "group" {
                let Some(syn::Expr::Path(p)) = expr else {
                    return err(span, "expected a type implementing Params");
                };
                let path = &p.path;
                el.groups
                    .push(quote!(<#path as ::xsvg_schema::Params>::SPEC.params));
                return Ok(());
            }
            let text = expr.and_then(expr_text);
            match key {
                "name" => el.name = text,
                "doc" => el.doc = text,
                "input" => el.input = text,
                "children" => el.children = text,
                other => return err(span, &format!("unknown element option `{other}`")),
            }
            Ok(())
        })?;
    }
    let el_name = el
        .name
        .clone()
        .unwrap_or_else(|| kebab_variant(&ast.ident));
    let el_doc = opt_str(&el.doc);
    let input_mode = mode_tokens(&el.input, "InputMode", &["none", "optional", "required"], ast.ident.span())?;
    let child_mode = mode_tokens(&el.children, "ChildMode", &["none", "container"], ast.ident.span())?;

    let mut specs = Vec::new();
    let mut reads = Vec::new();
    let mut groups: Vec<proc_macro2::TokenStream> = el.groups.clone();

    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named");
        let mut opts = ParamOpts::default();
        let mut values: Option<Vec<String>> = None;

        for attr in &field.attrs {
            if !attr.path().is_ident("param") {
                continue;
            }
            // `values = [...]` needs list syntax, so pull it out before the generic pass
            if let Meta::List(list) = &attr.meta {
                let raw = list.tokens.to_string();
                if raw.contains("values") {
                    if let Some(v) = extract_values(&raw) {
                        values = Some(v);
                    }
                }
            }
            parse_meta_list(&attr.meta, |key, expr, span| {
                match key {
                    "name" => opts.name = expr.and_then(expr_text),
                    "default" => opts.default = expr.and_then(expr_text),
                    "min" => opts.min = expr.and_then(expr_f64),
                    "max" => opts.max = expr.and_then(expr_f64),
                    "step" => opts.step = expr.and_then(expr_f64),
                    "doc" => opts.doc = expr.and_then(expr_text),
                    "superseded_by" => {
                        opts.superseded_by = expr
                            .and_then(expr_text)
                            .into_iter()
                            .flat_map(|s| {
                                s.split(',').map(|p| p.trim().to_string()).collect::<Vec<_>>()
                            })
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    "color" => opts.color = true,
                    "skip" => opts.skip = true,
                    "flatten" => opts.flatten = true,
                    "values" => {} // handled above
                    other => return err(span, &format!("unknown param option `{other}`")),
                }
                Ok(())
            })?;
        }
        if opts.skip {
            continue;
        }
        // A flattened field is another `Params` struct whose parameters this
        // element also accepts — one declaration of a shared block rather than
        // the same fields restated on every element that takes it.
        if opts.flatten {
            let ty = &field.ty;
            groups.push(quote!(<#ty as ::xsvg_schema::Params>::SPEC.params));
            reads.push(quote!(#ident: <#ty as ::xsvg_schema::Params>::from_attrs(src)));
            continue;
        }

        let name = opts.name.clone().unwrap_or_else(|| attr_name(ident));
        let ty = &field.ty;
        let inner = option_inner(ty);
        let base = inner.unwrap_or(ty);
        let optional = inner.is_some();

        // Kind + read expression per supported field type.
        let (kind, read) = if type_is(base, "f64") {
            let k = quote!(::xsvg_schema::ParamKind::Number);
            let r = if optional {
                quote!(src.attr(#name).and_then(::xsvg_schema::parse_num))
            } else {
                let d = default_f64(&opts, field.span())?;
                quote!(src.attr(#name).and_then(::xsvg_schema::parse_num).unwrap_or(#d))
            };
            (k, r)
        } else if type_is(base, "bool") {
            let k = quote!(::xsvg_schema::ParamKind::Bool);
            let r = if optional {
                quote!(src.attr(#name).and_then(::xsvg_schema::parse_bool))
            } else {
                let d = opts.default.as_deref() == Some("true");
                quote!(src.attr(#name).and_then(::xsvg_schema::parse_bool).unwrap_or(#d))
            };
            (k, r)
        } else if type_is(base, "String") {
            let k = if opts.color {
                quote!(::xsvg_schema::ParamKind::Color)
            } else {
                quote!(::xsvg_schema::ParamKind::Text)
            };
            let r = if optional {
                quote!(src.attr(#name).map(::std::string::ToString::to_string))
            } else {
                let d = opts.default.clone().unwrap_or_default();
                quote!(src.attr(#name).unwrap_or(#d).to_string())
            };
            (k, r)
        } else {
            // an enum parameter: its own derive supplies the spellings
            let k = quote!(::xsvg_schema::ParamKind::Enum);
            let r = if optional {
                quote!(src.attr(#name).and_then(<#base as ::xsvg_schema::ParamEnum>::parse))
            } else {
                quote!(
                    src.attr(#name)
                        .and_then(<#base as ::xsvg_schema::ParamEnum>::parse)
                        .unwrap_or_default()
                )
            };
            (k, r)
        };

        let values_tokens = match (&values, is_enum_field(base)) {
            (Some(v), _) => quote!(&[#(#v),*]),
            (None, true) => quote!(<#base as ::xsvg_schema::ParamEnum>::VALUES),
            _ => quote!(&[]),
        };
        let default_tokens = opt_str(&opts.default);
        let doc_tokens = opt_str(&opts.doc);
        let superseded = &opts.superseded_by;
        let min = opt_f64(opts.min);
        let max = opt_f64(opts.max);
        let step = opt_f64(opts.step);

        specs.push(quote! {
            ::xsvg_schema::ParamSpec {
                name: #name,
                kind: #kind,
                default: #default_tokens,
                min: #min,
                max: #max,
                step: #step,
                values: #values_tokens,
                doc: #doc_tokens,
                superseded_by: &[#(#superseded),*],
            }
        });
        reads.push(quote!(#ident: #read));
    }

    let ty = &ast.ident;
    let (imp, tys, whr) = ast.generics.split_for_impl();
    Ok(quote! {
        impl #imp ::xsvg_schema::Params for #ty #tys #whr {
            const SPEC: ::xsvg_schema::ElementSpec = ::xsvg_schema::ElementSpec {
                name: #el_name,
                doc: #el_doc,
                params: &[#(#specs),*],
                groups: &[#(#groups),*],
                input: #input_mode,
                children: #child_mode,
            };
            fn from_attrs(src: &dyn ::xsvg_schema::AttrSource) -> Self {
                Self { #(#reads),* }
            }
        }
    })
}

/// A non-`Option` field whose type is none of the primitives is taken to be an
/// enum parameter, so its spellings come from its own `ParamEnum` derive.
fn is_enum_field(base: &Type) -> bool {
    !(type_is(base, "f64") || type_is(base, "bool") || type_is(base, "String"))
}

fn default_f64(opts: &ParamOpts, span: proc_macro2::Span) -> syn::Result<f64> {
    match &opts.default {
        Some(d) => d.parse::<f64>().map_err(|_| {
            syn::Error::new(span, format!("default `{d}` is not a number"))
        }),
        // A required-looking number with no default would silently become 0.0,
        // which is rarely what an element means. Say so at compile time.
        None => err(
            span,
            "a non-Option number parameter needs `#[param(default = …)]` (or make it Option<f64>)",
        ),
    }
}

fn opt_str(v: &Option<String>) -> proc_macro2::TokenStream {
    match v {
        Some(s) => quote!(::core::option::Option::Some(#s)),
        None => quote!(::core::option::Option::None),
    }
}

fn opt_f64(v: Option<f64>) -> proc_macro2::TokenStream {
    match v {
        Some(f) => quote!(::core::option::Option::Some(#f)),
        None => quote!(::core::option::Option::None),
    }
}

fn mode_tokens(
    v: &Option<String>,
    ty: &str,
    allowed: &[&str],
    span: proc_macro2::Span,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = v.as_deref().unwrap_or(allowed[0]);
    if !allowed.contains(&name) {
        return err(span, &format!("expected one of {allowed:?}, got `{name}`"));
    }
    let ty: proc_macro2::TokenStream = ty.parse().unwrap();
    let variant = Ident::new(
        &{
            let mut c = name.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        },
        span,
    );
    Ok(quote!(::xsvg_schema::#ty::#variant))
}

/// Pull `values = ["a", "b"]` out of the raw attribute tokens — `syn`'s `Meta`
/// grammar has no place for a bracketed list on the value side.
fn extract_values(raw: &str) -> Option<Vec<String>> {
    let at = raw.find("values")?;
    let open = raw[at..].find('[')? + at;
    let close = raw[open..].find(']')? + open;
    let inner = &raw[open + 1..close];
    let out: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    (!out.is_empty()).then_some(out)
}

// ---- ParamEnum ------------------------------------------------------------

fn expand_param_enum(ast: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let Data::Enum(data) = &ast.data else {
        return err(ast.ident.span(), "ParamEnum can only be derived for an enum");
    };
    let mut names = Vec::new();
    let mut idents = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return err(v.ident.span(), "ParamEnum variants must be unit variants");
        }
        let mut spelling = kebab_variant(&v.ident);
        for attr in &v.attrs {
            if attr.path().is_ident("value") {
                if let Meta::List(list) = &attr.meta {
                    if let Ok(lit) = list.parse_args::<syn::LitStr>() {
                        spelling = lit.value();
                    }
                }
            }
        }
        names.push(spelling);
        idents.push(&v.ident);
    }
    let ty = &ast.ident;
    Ok(quote! {
        impl ::xsvg_schema::ParamEnum for #ty {
            const VALUES: &'static [&'static str] = &[#(#names),*];
            fn parse(s: &str) -> ::core::option::Option<Self> {
                match s.trim() {
                    #(#names => ::core::option::Option::Some(Self::#idents),)*
                    _ => ::core::option::Option::None,
                }
            }
            fn as_str(&self) -> &'static str {
                match self {
                    #(Self::#idents => #names,)*
                }
            }
        }
    })
}
