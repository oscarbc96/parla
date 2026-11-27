/// Defines this crate's `CATALOG` (embedded `i18n/*.ftl`) and its local `t!`.
/// Invoke once at the crate root before the modules that use `t!`:
///
/// ```ignore
/// i18n::namespace!("components");
/// // t!(cx, "sidebar-unreads"); t!(cx, "thread-replies", count = n);
/// ```
///
/// `cx` is anything that derefs to a [`Translate`](crate::Translate): a
/// `Localizer`, the app (and so any `Context`) once the global is installed.
#[macro_export]
macro_rules! namespace {
    ($ns:literal) => {
        $crate::namespace!($ns, folder = "i18n");
    };
    ($ns:literal, folder = $folder:literal) => {
        $crate::namespace!(@define $ns, $folder, $);
    };
    (@define $ns:literal, $folder:literal, $d:tt) => {
        #[derive($crate::rust_embed::RustEmbed)]
        #[crate_path = "i18n::rust_embed"]
        #[folder = $folder]
        struct I18nEmbedded;

        fn i18n_embedded_get(path: &str) -> Option<::std::borrow::Cow<'static, [u8]>> {
            <I18nEmbedded as $crate::rust_embed::Embed>::get(path).map(|f| f.data)
        }

        pub(crate) static CATALOG: $crate::Catalog =
            $crate::Catalog::new($ns, i18n_embedded_get);

        #[allow(unused_macros)]
        macro_rules! t {
            ($d cx:expr, $d id:literal $d(, $d k:ident = $d v:expr)* $d(,)?) => {{
                #[allow(unused_mut)]
                let mut args = $crate::FluentArgs::new();
                $d(args.set(stringify!($d k), $crate::Arg::from($d v).to_value());)*
                let _ = &args;
                {
                    use $crate::Translate as _;
                    ($d cx).localizer().format($ns, $d id, Some(&args))
                }
            }};
        }
    };
}
