use super::*;

fn run_large_stack_dom_test<F>(thread_name: &'static str, test: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(test)
        .expect("large-stack DOM test thread should spawn")
        .join()
        .expect("large-stack DOM test thread should finish");
}

mod detached_adoption;
mod detached_documents;
mod dom_parser;
mod element_construction;
mod elements_and_events;
mod files_and_transfer;
mod pointer_and_mouse;
mod ranges;
mod rendered_text;
mod style_and_geometry;
mod tree_mutations;
mod xpath_and_selectors;
