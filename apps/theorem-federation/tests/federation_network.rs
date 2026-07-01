use std::io;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::{store::mem::MemStore, ticket::BlobTicket, BlobsProtocol};
use iroh_gossip::{api::Event as GossipEvent, Gossip};
use rustyred_thg_core::{ActorId, InMemoryThgExecutor, VersionVector};
use serde_json::json;
use tempfile::TempDir;
use theorem_copresence::{
    PeerConfig, PeerEvent, Presence, PresenceKind, StructuredOp, SubstratePeer,
};
use theorem_federation::blob::{BlobResolver, PayloadKind};
use theorem_federation::delta::{read_frame, write_frame, FederationFrame};
use theorem_federation::gossip::{
    awareness_topic_id, decode_awareness, encode_awareness, AwarenessMessage,
};
use theorem_federation::identity::FEDERATION_ALPN;
use theorem_federation::transport::{apply_inbound_frame, InboundFrameKind};
use theorem_federation::trust::TrustPolicy;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test(flavor = "current_thread")]
async fn authenticated_quic_stream_applies_structured_and_text_delta() -> TestResult {
    let dir = TempDir::new()?;
    let scope = "repo:theorem:federation";
    let mut source_peer = substrate_peer("source", scope, &dir, "source")?;

    source_peer.apply_structured(StructuredOp::SetObjectProperty {
        object_id: "doc:spec".to_string(),
        labels: vec!["Spec".to_string()],
        key: "title".to_string(),
        value: json!("Iroh federation"),
    })?;
    source_peer.push_text("doc:spec:body", "delta over authenticated quic")?;

    let structured = FederationFrame::StructuredDelta {
        scope: scope.to_string(),
        batch: source_peer.delta_since(&VersionVector::default()),
    };
    let text_state_vector =
        substrate_peer("empty", scope, &dir, "empty")?.text_state_vector("doc:spec:body")?;
    let text = FederationFrame::TextUpdate {
        scope: scope.to_string(),
        region_id: "doc:spec:body".to_string(),
        state_vector_v1: text_state_vector.clone(),
        update_v1: source_peer.text_update_since("doc:spec:body", &text_state_vector)?,
    };

    let server_endpoint = local_endpoint(vec![FEDERATION_ALPN.to_vec()], None).await?;
    let server_addr = server_endpoint.addr();
    let client_endpoint = local_endpoint(vec![FEDERATION_ALPN.to_vec()], None).await?;
    let client_id = client_endpoint.id();
    let server_id = server_endpoint.id();
    let server_dir = dir.path().join("receiver");

    let server = async move {
        let mut receiver_peer = substrate_peer_at("receiver", scope, server_dir)?;
        receiver_peer.apply_structured(StructuredOp::SetObjectProperty {
            object_id: "doc:spec".to_string(),
            labels: vec!["Spec".to_string()],
            key: "server_status".to_string(),
            value: json!("edited concurrently"),
        })?;
        let trust = TrustPolicy::default();
        let incoming = timeout(server_endpoint.accept())
            .await?
            .ok_or_else(|| io_error("server endpoint closed before accepting"))?;
        let conn = timeout(
            incoming
                .accept()
                .map_err(theorem_federation::FederationError::iroh)?,
        )
        .await??;
        assert_eq!(conn.remote_id(), client_id);
        let (mut send, mut recv) = timeout(conn.accept_bi()).await??;

        let first = read_frame(&mut recv).await?;
        let first_receipt = apply_inbound_frame(
            &mut receiver_peer,
            &trust,
            &conn.remote_id().to_string(),
            first,
        )?;
        let second = read_frame(&mut recv).await?;
        let second_receipt = apply_inbound_frame(
            &mut receiver_peer,
            &trust,
            &conn.remote_id().to_string(),
            second,
        )?;

        write_frame(
            &mut send,
            &FederationFrame::StructuredDelta {
                scope: scope.to_string(),
                batch: receiver_peer.delta_since(&VersionVector::default()),
            },
        )
        .await?;
        send.finish()
            .map_err(theorem_federation::FederationError::iroh)?;
        let title = receiver_peer
            .graph_node("doc:spec")
            .and_then(|node| node.properties.get("title").cloned());
        let body = receiver_peer.text_region_contents("doc:spec:body");
        conn.closed().await;
        server_endpoint.close().await;

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
            title,
            body,
            first_receipt,
            second_receipt,
        ))
    };

    let started = Instant::now();
    let client = async move {
        let mut source_peer = source_peer;
        let trust = TrustPolicy::default();
        let conn = client_endpoint
            .connect(server_addr, FEDERATION_ALPN)
            .await?;
        assert_eq!(conn.remote_id(), server_id);
        let (mut send, mut recv) = timeout(conn.open_bi()).await??;
        write_frame(&mut send, &structured).await?;
        write_frame(&mut send, &text).await?;
        send.finish()
            .map_err(theorem_federation::FederationError::iroh)?;
        let ack = timeout(read_frame(&mut recv)).await??;
        let ack_receipt =
            apply_inbound_frame(&mut source_peer, &trust, &conn.remote_id().to_string(), ack)?;
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "local QUIC delta exchange exceeded 1s: {:?}",
            started.elapsed()
        );
        let source_status = source_peer
            .graph_node("doc:spec")
            .and_then(|node| node.properties.get("server_status").cloned());
        conn.close(0u32.into(), b"done");
        client_endpoint.close().await;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((source_status, ack_receipt))
    };

    let (server_result, client_result) = tokio::join!(server, client);
    let (title, body, structured_receipt, text_receipt) = server_result?;
    let (source_status, ack_receipt) = client_result?;
    assert_eq!(structured_receipt.kind, InboundFrameKind::StructuredDelta);
    assert_eq!(structured_receipt.applied_structured, 1);
    assert_eq!(text_receipt.kind, InboundFrameKind::TextUpdate);
    assert_eq!(text_receipt.applied_text, 1);
    assert_eq!(ack_receipt.kind, InboundFrameKind::StructuredDelta);
    assert_eq!(title, Some(json!("Iroh federation")));
    assert_eq!(source_status, Some(json!("edited concurrently")));
    assert_eq!(body.as_deref(), Some("delta over authenticated quic"));

    Ok(())
}

#[tokio::test]
async fn iroh_blobs_transfers_pack_bytes_and_records_dual_hash_boundary() -> TestResult {
    let dir = TempDir::new()?;
    let provider_endpoint = local_endpoint(Vec::new(), None).await?;
    let provider_store = MemStore::new();
    let provider_protocol = BlobsProtocol::new(&provider_store, None);
    let provider_router = Router::builder(provider_endpoint)
        .accept(iroh_blobs::ALPN, provider_protocol)
        .spawn();

    let bytes = br#"{"pack":"cold graph version","version":42}"#;
    let tag = provider_store.add_slice(bytes).await?;
    let ticket = BlobTicket::new(provider_router.endpoint().addr(), tag.hash, tag.format);

    let client_endpoint = local_endpoint(Vec::new(), None).await?;
    let lookup = MemoryLookup::new();
    lookup.add_endpoint_info(ticket.addr().clone());
    client_endpoint.address_lookup()?.add(lookup);
    let client_store = MemStore::new();
    client_store
        .downloader(&client_endpoint)
        .download(ticket.hash(), Some(ticket.addr().id))
        .await?;
    let received = client_store.get_bytes(ticket.hash()).await?;

    let mut resolver = BlobResolver::open(dir.path().join("resolver.json"))?;
    let mapping = resolver.record_bytes_from_peer(
        &ticket.addr().id.to_string(),
        &TrustPolicy::default(),
        &received,
        PayloadKind::ProllyPack,
    )?;

    assert_eq!(received.as_ref(), bytes);
    assert!(mapping.sha256.starts_with("sha256:"));
    assert!(mapping.blake3.starts_with("blake3:"));
    assert_ne!(mapping.sha256, mapping.blake3);

    client_endpoint.close().await;
    provider_router.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn three_peer_gossip_mesh_delivers_awareness_payload() -> TestResult {
    let topic = awareness_topic_id("Travis-Gilbert", "repo:theorem");
    let lookup = MemoryLookup::new();
    let endpoint1 = local_endpoint(Vec::new(), Some(lookup.clone())).await?;
    let endpoint2 = local_endpoint(Vec::new(), Some(lookup.clone())).await?;
    let endpoint3 = local_endpoint(Vec::new(), Some(lookup.clone())).await?;

    let gossip1 = Gossip::builder().spawn(endpoint1.clone());
    let gossip2 = Gossip::builder().spawn(endpoint2.clone());
    let gossip3 = Gossip::builder().spawn(endpoint3.clone());
    let router1 = Router::builder(endpoint1)
        .accept(iroh_gossip::ALPN, gossip1.clone())
        .spawn();
    let router2 = Router::builder(endpoint2)
        .accept(iroh_gossip::ALPN, gossip2.clone())
        .spawn();
    let router3 = Router::builder(endpoint3)
        .accept(iroh_gossip::ALPN, gossip3.clone())
        .spawn();

    let addr1 = router1.endpoint().addr();
    let addr2 = router2.endpoint().addr();
    let id1 = addr1.id;
    let id2 = addr2.id;
    lookup.add_endpoint_info(addr1);
    lookup.add_endpoint_info(addr2);
    lookup.add_endpoint_info(router3.endpoint().addr());

    let (sub1, sub2, sub3) = timeout(async {
        tokio::try_join!(
            gossip1.subscribe_and_join(topic, vec![]),
            gossip2.subscribe_and_join(topic, vec![id1]),
            gossip3.subscribe_and_join(topic, vec![id2]),
        )
    })
    .await??;
    let (sender1, _receiver1) = sub1.split();
    let (_sender2, mut receiver2) = sub2.split();
    let (_sender3, mut receiver3) = sub3.split();

    let message = AwarenessMessage {
        tenant: "Travis-Gilbert".to_string(),
        scope: "repo:theorem".to_string(),
        event: PeerEvent::Presence {
            cursor: 7,
            presence: Presence {
                actor: "codex".to_string().into(),
                scope: "repo:theorem".to_string(),
                focus_region: Some("apps/theorem-federation".to_string()),
                cursor: None,
                label: "codex".to_string(),
                kind: PresenceKind::Agent,
            },
        },
    };
    sender1
        .broadcast(encode_awareness(&message)?.into())
        .await?;

    wait_for_awareness(&mut receiver2, &message).await?;
    wait_for_awareness(&mut receiver3, &message).await?;

    router1.shutdown().await?;
    router2.shutdown().await?;
    router3.shutdown().await?;
    Ok(())
}

#[test]
fn trust_rejection_prevents_delta_and_blob_acceptance() -> TestResult {
    let dir = TempDir::new()?;
    let mut source_peer = substrate_peer("source", "repo", &dir, "source")?;
    source_peer.apply_structured(StructuredOp::SetObjectProperty {
        object_id: "doc:blocked".to_string(),
        labels: vec!["Spec".to_string()],
        key: "title".to_string(),
        value: json!("should not land"),
    })?;

    let mut receiver_peer = substrate_peer("receiver", "repo", &dir, "receiver")?;
    let mut trust = TrustPolicy::with_floor(0.95);
    for _ in 0..5 {
        trust.record_outcome("endpoint:bad", false);
    }
    let frame = FederationFrame::StructuredDelta {
        scope: "repo".to_string(),
        batch: source_peer.delta_since(&VersionVector::default()),
    };

    assert!(apply_inbound_frame(&mut receiver_peer, &trust, "endpoint:bad", frame).is_err());
    assert!(receiver_peer.graph_node("doc:blocked").is_none());

    let mut resolver = BlobResolver::open(dir.path().join("resolver.json"))?;
    assert!(resolver
        .record_bytes_from_peer(
            "endpoint:bad",
            &trust,
            br#"{"blocked":true}"#,
            PayloadKind::ColdObject,
        )
        .is_err());
    assert!(resolver.mappings().is_empty());

    Ok(())
}

async fn wait_for_awareness(
    receiver: &mut iroh_gossip::api::GossipReceiver,
    expected: &AwarenessMessage,
) -> TestResult {
    loop {
        let event = timeout(receiver.next())
            .await?
            .ok_or_else(|| io_error("gossip stream closed before awareness message"))??;
        match event {
            GossipEvent::Received(message) => {
                assert_eq!(decode_awareness(&message.content)?, *expected);
                return Ok(());
            }
            GossipEvent::Lagged => return Err(io_error("gossip receiver lagged").into()),
            GossipEvent::NeighborUp(_) | GossipEvent::NeighborDown(_) => {}
        }
    }
}

fn substrate_peer(
    actor: &str,
    scope: &str,
    dir: &TempDir,
    name: &str,
) -> TestResult<SubstratePeer> {
    substrate_peer_at(actor, scope, dir.path().join(name))
}

fn substrate_peer_at(
    actor: &str,
    scope: &str,
    data_dir: impl Into<std::path::PathBuf>,
) -> TestResult<SubstratePeer> {
    Ok(SubstratePeer::try_new(
        InMemoryThgExecutor::new(),
        PeerConfig::new(ActorId::from_label(actor), scope).with_data_dir(data_dir),
    )?)
}

async fn timeout<T>(future: impl std::future::Future<Output = T>) -> TestResult<T> {
    Ok(tokio::time::timeout(Duration::from_secs(10), future).await?)
}

async fn local_endpoint(alpns: Vec<Vec<u8>>, lookup: Option<MemoryLookup>) -> TestResult<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))?;
    if let Some(lookup) = lookup {
        builder = builder.address_lookup(lookup);
    }
    if !alpns.is_empty() {
        builder = builder.alpns(alpns);
    }
    Ok(builder.bind().await?)
}

fn io_error(message: &str) -> io::Error {
    io::Error::other(message)
}
