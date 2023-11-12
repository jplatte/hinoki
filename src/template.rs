use minijinja::UndefinedBehavior;

pub(crate) mod functions;

pub(crate) fn environment<'a>() -> minijinja::Environment<'a> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_function("load_data", functions::load_data);

    #[cfg(feature = "datetime")]
    {
        use minijinja_contrib::filters as contrib_filters;

        env.add_filter("dateformat", contrib_filters::dateformat);
        env.add_filter("datetimeformat", contrib_filters::datetimeformat);
        env.add_filter("timeformat", contrib_filters::timeformat);
    }

    env
}
