use theorem_copresence::{PeerEvent, Presence, PresenceKind};
use theorem_federation::blob::{
    blake3_transfer_link, sha256_identity, verify_transfer_boundary, BlobResolver, PayloadKind,
};
use theorem_federation::delta::{
    decode_frame, encode_frame, read_frame, write_frame, FederationFrame,
};
use theorem_federation::driver::parse_peers;
use theorem_federation::gossip::{
    awareness_topic_id, decode_awareness, encode_awareness, AwarenessMessage,
};
use theorem_federation::identity::{identity_status, IdentityConfig};
use theorem_federation::trust::TrustPolicy;

#[test]
fn identity_is_stable_for_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let config = IdentityConfig::new(dir.path());

    let first = identity_status(&config).unwrap();
    let second = identity_status(&config).unwrap();

    assert_eq!(first.endpoint_id, second.endpoint_id);
    assert!(first.key_path.exists());
}

#[test]
fn resolver_keeps_sha256_identity_and_blake3_transfer_link_separate() {
    let dir = tempfile::tempdir().unwrap();
    let mut resolver = BlobResolver::open(dir.path().join("resolver.json")).unwrap();
    let bytes = br#"{"pack":"source bytes"}"#;

    let mapping = resolver
        .record_bytes_from_peer(
            "endpoint:peer",
            &TrustPolicy::default(),
            bytes,
            PayloadKind::ProllyPack,
        )
        .unwrap();

    assert_eq!(mapping.sha256, sha256_identity(bytes));
    assert_eq!(mapping.blake3, blake3_transfer_link(bytes));
    assert!(verify_transfer_boundary(bytes, &mapping));
    assert_eq!(
        resolver.resolve_blake3(&mapping.sha256),
        Some(mapping.blake3.as_str())
    );
    assert_eq!(
        resolver.resolve_sha256(&mapping.blake3),
        Some(mapping.sha256.as_str())
    );
    assert!(!verify_transfer_boundary(b"changed", &mapping));
}

#[tokio::test]
async fn delta_frame_round_trips_over_length_prefixed_stream() {
    let frame = FederationFrame::TextUpdate {
        scope: "tenant:scope".to_string(),
        region_id: "region:a".to_string(),
        state_vector_v1: vec![1, 2],
        update_v1: vec![3, 4, 5],
    };
    let encoded = encode_frame(&frame).unwrap();
    assert_eq!(decode_frame(&encoded).unwrap(), frame);

    let (mut tx, mut rx) = tokio::io::duplex(1024);
    write_frame(&mut tx, &frame).await.unwrap();
    let received = read_frame(&mut rx).await.unwrap();
    assert_eq!(received, frame);
}

#[test]
fn awareness_topic_and_payload_are_deterministic() {
    let presence = Presence {
        actor: "codex".to_string().into(),
        scope: "repo".to_string(),
        focus_region: None,
        cursor: None,
        label: "codex".to_string(),
        kind: PresenceKind::Agent,
    };
    let message = AwarenessMessage {
        tenant: "Travis-Gilbert".to_string(),
        scope: "repo".to_string(),
        event: PeerEvent::Presence {
            cursor: 1,
            presence,
        },
    };

    assert_eq!(
        awareness_topic_id("Travis-Gilbert", "repo"),
        awareness_topic_id("Travis-Gilbert", "repo")
    );
    let encoded = encode_awareness(&message).unwrap();
    assert_eq!(decode_awareness(&encoded).unwrap(), message);
}

#[test]
fn trust_policy_gates_below_floor_without_merge_changes() {
    let mut policy = TrustPolicy::with_floor(0.5);
    let peer = "endpoint:peer";

    for _ in 0..4 {
        policy.record_outcome(peer, false);
    }

    assert!(!policy.allow_inbound(peer));
    assert!(policy.require_inbound(peer).is_err());
    policy.set_floor(0.2);
    assert!(policy.require_inbound(peer).is_ok());
}

#[test]
fn peers_parse_endpoint_and_optional_relay() {
    let peers = parse_peers(Some("abc@https://relay.example, def"));

    assert_eq!(peers.len(), 2);
    assert_eq!(peers[0].endpoint_id, "abc");
    assert_eq!(peers[0].relay_url.as_deref(), Some("https://relay.example"));
    assert_eq!(peers[1].endpoint_id, "def");
    assert_eq!(peers[1].relay_url, None);
}
