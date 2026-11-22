use swm_api::{Client, EventTopic};

fn main() -> swm_api::Result<()> {
    let client = Client::connect()?;
    let mut subscription =
        client.subscribe([EventTopic::Outputs, EventTopic::Nodes, EventTopic::Config])?;

    println!(
        "snapshot {}: {} outputs, {} nodes",
        subscription.initial.sequence,
        subscription.initial.outputs.len(),
        subscription.initial.nodes.len()
    );
    for event in &mut subscription.events {
        println!("{:?}", event?);
    }
    Ok(())
}
