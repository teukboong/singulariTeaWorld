mod runtime;
mod surface;

fn main() -> anyhow::Result<()> {
    surface::cli::run()
}
