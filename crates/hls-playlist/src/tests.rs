use super::*;

fn seg(pdt: &str, uri: &str, title: &str) -> String {
    format!("#EXT-X-PROGRAM-DATE-TIME:{pdt}\n#EXTINF:2.000,{title}\n{uri}\n")
}

const HEAD: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n";

fn playlist(seq: u64, extra_tags: &str, segs: &[(&str, &str)]) -> String {
    let mut s = format!("{HEAD}#EXT-X-MEDIA-SEQUENCE:{seq}\n{extra_tags}");
    for (pdt, uri) in segs {
        s.push_str(&seg(pdt, uri, "live"));
    }
    s
}

#[test]
fn rfc3339() {
    assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_rfc3339_ms("2000-01-01T00:00:00.500Z"), Some(946_684_800_500));
    assert_eq!(parse_rfc3339_ms("2000-01-01T01:00:00+01:00"), Some(946_684_800_000));
    assert_eq!(parse_rfc3339_ms("n'importe quoi"), None);
}

#[test]
fn attrs_with_quoted_commas() {
    let a = parse_attrs(r#"ID="stitched-ad-1,x",CLASS="twitch-stitched-ad",DURATION=15.0"#);
    assert_eq!(attr(&a, "ID"), Some("stitched-ad-1,x"));
    assert_eq!(attr(&a, "DURATION"), Some("15.0"));
}

#[test]
fn trigger_daterange_is_not_an_ad() {
    let t = playlist(10, "#EXT-X-DATERANGE:ID=\"trigger\",CLASS=\"twitch-trigger\",START-DATE=\"2026-11-18T17:00:00Z\"\n", &[("2026-11-18T17:00:00Z", "a.ts")]);
    let p = parse_media(&t);
    assert!(!p.has_ads());
    assert_eq!(p.segments.len(), 1);
}

#[test]
fn detects_stitched_ad_by_daterange() {
    let dr = "#EXT-X-DATERANGE:ID=\"stitched-ad-1\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-11-18T17:00:02Z\",DURATION=4.0,X-TV-TWITCH-AD-ROLL-TYPE=\"MIDROLL\"\n";
    let t = playlist(
        100,
        dr,
        &[
            ("2026-11-18T17:00:00Z", "live0.ts"),
            ("2026-11-18T17:00:02Z", "ad1.ts"),
            ("2026-11-18T17:00:04Z", "ad2.ts"),
            ("2026-11-18T17:00:06Z", "live1.ts"), // = fin exacte de la pub : contenu
        ],
    );
    let p = parse_media(&t);
    let flags: Vec<bool> = p.segments.iter().map(|s| s.is_ad).collect();
    assert_eq!(flags, vec![false, true, true, false]);
    assert!(p.has_ads());
    assert!(p.ad_signals.contains(&"daterange:twitch-stitched-ad"));
}

#[test]
fn detects_cue_out_and_legacy_title() {
    let mut t = format!("{HEAD}#EXT-X-MEDIA-SEQUENCE:1\n");
    t.push_str(&seg("2026-11-18T17:00:00Z", "l.ts", "live"));
    t.push_str("#EXT-X-CUE-OUT:DURATION=4\n");
    t.push_str(&seg("2026-11-18T17:00:02Z", "c1.ts", "live"));
    t.push_str("#EXT-X-CUE-IN\n");
    t.push_str(&seg("2026-11-18T17:00:04Z", "l2.ts", "live"));
    t.push_str(&seg("2026-11-18T17:00:06Z", "am.ts", "Amazon"));
    let flags: Vec<bool> = parse_media(&t).segments.iter().map(|s| s.is_ad).collect();
    assert_eq!(flags, vec![false, true, false, true]);
}

#[test]
fn live_filter_keeps_stable_numbering_across_an_ad_break() {
    let mut f = LivePlaylistFilter::new(6);

    // 1) playlist propre initiale
    let t1 = playlist(100, "", &[("2026-11-18T17:00:00Z", "s0.ts"), ("2026-11-18T17:00:02Z", "s1.ts")]);
    let o1 = f.update(&t1);
    assert_eq!(o1.removed, 0);
    assert!(o1.playlist.contains("#EXT-X-MEDIA-SEQUENCE:0\n"));
    assert!(o1.playlist.contains("s0.ts") && o1.playlist.contains("s1.ts"));

    // 2) la source ajoute 2 segments pub puis 1 segment live (numérotation source décalée)
    let dr = "#EXT-X-DATERANGE:ID=\"stitched-ad-9\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-11-18T17:00:04Z\",DURATION=4.0\n";
    let t2 = playlist(
        101,
        dr,
        &[("2026-11-18T17:00:02Z", "s1.ts"), ("2026-11-18T17:00:04Z", "ad1.ts"), ("2026-11-18T17:00:06Z", "ad2.ts"), ("2026-11-18T17:00:08Z", "s4.ts")],
    );
    let o2 = f.update(&t2);
    assert_eq!(o2.removed, 2);
    assert!(o2.ad_active);
    assert!(!o2.playlist.contains("ad1.ts") && !o2.playlist.contains("ad2.ts"));
    assert!(o2.playlist.contains("s0.ts") && o2.playlist.contains("s1.ts") && o2.playlist.contains("s4.ts"));
    // s1 n'est pas dupliqué, et une rupture de continuité précède s4
    assert_eq!(o2.playlist.matches("s1.ts").count(), 1);
    let before_s4 = &o2.playlist[..o2.playlist.find("s4.ts").unwrap()];
    assert!(before_s4.trim_end().ends_with("#EXTINF:2.000,live") && before_s4.contains("#EXT-X-DISCONTINUITY"));
    assert!(o2.playlist.contains("#EXT-X-MEDIA-SEQUENCE:0\n"));

    // 3) la fenêtre glisse : MEDIA-SEQUENCE ne recule jamais
    let mut seqs = vec![0u64];
    let mut t = 10;
    for i in 0..10 {
        let pdt = format!("2026-11-18T17:00:{:02}Z", t);
        let uri = format!("n{i}.ts");
        let o = f.update(&playlist(110 + i, "", &[(pdt.as_str(), uri.as_str())]));
        let seq: u64 = o.playlist.lines().find_map(|l| l.strip_prefix("#EXT-X-MEDIA-SEQUENCE:")).unwrap().parse().unwrap();
        seqs.push(seq);
        t += 2;
    }
    assert!(seqs.windows(2).all(|w| w[0] <= w[1]), "{seqs:?}");
    assert!(*seqs.last().unwrap() > 0);
}

#[test]
fn filter_is_session_agnostic_via_program_date_time() {
    // Un playlist « backup » avec une autre numérotation de séquence ne doit pas rejouer l'existant.
    let mut f = LivePlaylistFilter::new(6);
    f.update(&playlist(500, "", &[("2026-11-18T17:00:00Z", "a.ts"), ("2026-11-18T17:00:02Z", "b.ts")]));
    let o = f.update(&playlist(7, "", &[("2026-11-18T17:00:02Z", "b2.ts"), ("2026-11-18T17:00:04Z", "c2.ts")]));
    assert!(!o.playlist.contains("b2.ts"));
    assert!(o.playlist.contains("c2.ts"));
}

#[test]
fn master_and_variant_choice() {
    let m = "#EXTM3U\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
#EXT-X-STREAM-INF:PROGRAM-ID=1,BANDWIDTH=8000000,CODECS=\"avc1.64002A,mp4a.40.2\",VIDEO=\"chunked\"\nhttps://x/chunked.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"720p60\",NAME=\"720p60\",AUTOSELECT=YES,DEFAULT=NO\n\
#EXT-X-STREAM-INF:PROGRAM-ID=1,BANDWIDTH=3500000,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"720p60\"\nhttps://x/720.m3u8\n\
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"audio_only\",NAME=\"audio_only\",AUTOSELECT=NO,DEFAULT=NO\n\
#EXT-X-STREAM-INF:PROGRAM-ID=1,BANDWIDTH=160000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\nhttps://x/audio.m3u8\n";
    let v = parse_master(m);
    assert_eq!(v.len(), 3);
    assert_eq!(pick_variant(&v, "best").unwrap().url, "https://x/chunked.m3u8");
    assert_eq!(pick_variant(&v, "720p60").unwrap().url, "https://x/720.m3u8");
    assert_eq!(pick_variant(&v, "audio_only").unwrap().url, "https://x/audio.m3u8");
    assert_eq!(pick_variant(&v, "1080p").unwrap().url, "https://x/chunked.m3u8"); // via NAME
}

#[test]
fn strip_can_be_toggled_live_without_renumbering() {
    let dr = "#EXT-X-DATERANGE:ID=\"stitched-ad-1\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-11-18T17:00:02Z\",DURATION=2.0\n";
    let mut f = LivePlaylistFilter::new(6);
    f.set_strip(false);
    let o = f.update(&playlist(1, dr, &[("2026-11-18T17:00:00Z", "l0.ts"), ("2026-11-18T17:00:02Z", "ad.ts")]));
    assert_eq!(o.removed, 0);
    assert!(o.playlist.contains("ad.ts")); // désactivé : la pub passe
    f.set_strip(true);
    let o = f.update(&playlist(2, dr, &[("2026-11-18T17:00:02Z", "ad.ts"), ("2026-11-18T17:00:04Z", "l1.ts")]));
    assert!(o.playlist.contains("l1.ts") && o.playlist.matches("ad.ts").count() == 1);
}
