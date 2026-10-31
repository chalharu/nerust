#[path = "compile_fail/self_arguments/main.rs"]
mod probe;

#[test]
fn command_arguments_keep_the_source_model_self_context() {
    probe::main();
}
