use super::*;

// The same unit table drives the namespace factories and CSSUnitValue's
// constructor, including the public Q/Hz/kHz names and lowercase unit values.
macro_rules! css_numeric_factories {
    ($($field:ident: $method:literal => $unit:literal),* $(,)?) => {
        pub(super) const VALID_UNIT_NAMES: &[&str] = &[$($unit),*];

        #[derive(Default, WebApiObject)]
        #[webapi(fragment, enumerable)]
        struct CssNumericFactoriesDeclaration {
            $(
                #[webapi(
                    method = $method,
                    callback = css_numeric_factory_callback,
                    data = v8_string(scope, $unit).expect("CSS numeric unit"),
                    length = 1
                )]
                $field: (),
            )*
        }
    };
}

css_numeric_factories! {
    number: "number" => "number",
    percent: "percent" => "percent",
    cap: "cap" => "cap",
    ch: "ch" => "ch",
    em: "em" => "em",
    ex: "ex" => "ex",
    ic: "ic" => "ic",
    lh: "lh" => "lh",
    rcap: "rcap" => "rcap",
    rch: "rch" => "rch",
    rem: "rem" => "rem",
    rex: "rex" => "rex",
    ric: "ric" => "ric",
    rlh: "rlh" => "rlh",
    vw: "vw" => "vw",
    vh: "vh" => "vh",
    vi: "vi" => "vi",
    vb: "vb" => "vb",
    vmin: "vmin" => "vmin",
    vmax: "vmax" => "vmax",
    svw: "svw" => "svw",
    svh: "svh" => "svh",
    svi: "svi" => "svi",
    svb: "svb" => "svb",
    svmin: "svmin" => "svmin",
    svmax: "svmax" => "svmax",
    lvw: "lvw" => "lvw",
    lvh: "lvh" => "lvh",
    lvi: "lvi" => "lvi",
    lvb: "lvb" => "lvb",
    lvmin: "lvmin" => "lvmin",
    lvmax: "lvmax" => "lvmax",
    dvw: "dvw" => "dvw",
    dvh: "dvh" => "dvh",
    dvi: "dvi" => "dvi",
    dvb: "dvb" => "dvb",
    dvmin: "dvmin" => "dvmin",
    dvmax: "dvmax" => "dvmax",
    cqw: "cqw" => "cqw",
    cqh: "cqh" => "cqh",
    cqi: "cqi" => "cqi",
    cqb: "cqb" => "cqb",
    cqmin: "cqmin" => "cqmin",
    cqmax: "cqmax" => "cqmax",
    cm: "cm" => "cm",
    mm: "mm" => "mm",
    q: "Q" => "q",
    inches: "in" => "in",
    pt: "pt" => "pt",
    pc: "pc" => "pc",
    px: "px" => "px",
    deg: "deg" => "deg",
    grad: "grad" => "grad",
    rad: "rad" => "rad",
    turn: "turn" => "turn",
    s: "s" => "s",
    ms: "ms" => "ms",
    hz: "Hz" => "hz",
    khz: "kHz" => "khz",
    dpi: "dpi" => "dpi",
    dpcm: "dpcm" => "dpcm",
    dppx: "dppx" => "dppx",
    fr: "fr" => "fr",
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSS numeric factory")]
struct CssNumericFactoryArgs {
    #[webidl(required, converter = "double")]
    value: f64,
}

pub(in crate::context_bootstrap::css_runtime) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    namespace: v8::Local<'s, v8::Object>,
) {
    CssNumericFactoriesDeclaration::default()
        .initialize(scope, namespace)
        .expect("CSS numeric factories should initialize");
}

fn css_numeric_factory_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CssNumericFactoryArgs>(scope, &args) else {
        return;
    };
    let unit = v8::Local::<v8::String>::try_from(args.data())
        .expect("CSS numeric factory unit")
        .to_rust_string_lossy(scope);
    rv.set(values::unit_value(scope, parsed.value, unit).into());
}
