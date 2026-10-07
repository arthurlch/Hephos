use rivet::agent::{Agent, Model};

pub fn build<M: Model>(model: M) -> Agent<M> {
    Agent::new(model).system("Write a single warm sentence welcoming a new user.")
}
