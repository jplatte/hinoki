mod functions;

pub(crate) fn environment<'a>() -> minijinja::Environment<'a> {
    let mut env = minijinja::Environment::new();
    env.add_function("load_data", functions::load_data);
    env
}
