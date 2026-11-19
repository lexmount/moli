use moli_webidl as webidl;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IntersectionObserverInit")]
pub struct IntersectionObserverInitMembers<Root>
where
    Root: for<'scope> webidl::WebIdlConverter<'scope, Options = ()>,
{
    // The derive converts members in lexical order of their final JavaScript names.
    #[webidl(default = 0)]
    pub delay: i32,
    #[webidl(legacy_nullish, converter = "raw")]
    pub root: Option<Root>,
    #[webidl(default = "0px")]
    pub root_margin: String,
    #[webidl(default = "0px")]
    pub scroll_margin: String,
    #[webidl(with = intersection_observer_threshold_member)]
    pub threshold: Vec<f64>,
    #[webidl(default = false)]
    pub track_visibility: bool,
}

fn intersection_observer_threshold_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Result<Vec<f64>, webidl::WebIdlError> {
    let context = webidl::Context::member("IntersectionObserverInit", member);
    let Some(value) =
        webidl::optional_member::<v8::Local<'s, v8::Value>>(scope, object, member, context)?
    else {
        return Ok(vec![0.0]);
    };

    // Convert the union at its dictionary position, reading @@iterator once.
    if let Some(sequence) =
        webidl::convert_optional_sequence::<webidl::Double>(scope, value, context, &())?
    {
        Ok(sequence.0.into_iter().map(|value| value.0).collect())
    } else {
        Ok(vec![
            webidl::convert::<webidl::Double>(scope, value, context)?.0,
        ])
    }
}
