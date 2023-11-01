use minijinja::UndefinedBehavior;

mod functions;

pub(crate) fn environment<'a>() -> minijinja::Environment<'a> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_function("load_data", functions::load_data);
    env
}
