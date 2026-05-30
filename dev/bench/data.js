window.BENCHMARK_DATA = {
  "lastUpdate": 1780125995387,
  "repoUrl": "https://github.com/jdh8/pons-dds",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "committer": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "distinct": true,
          "id": "a33484a0c483e1a1c3f212e2f86ba25f8da02401",
          "message": "ci: add docs and release test jobs; fix clippy collapsible-if warnings",
          "timestamp": "2026-05-30T04:53:11+08:00",
          "tree_id": "59628e0b530ec74460ca4d8916f9be2a5b2991eb",
          "url": "https://github.com/jdh8/dds-rs/commit/a33484a0c483e1a1c3f212e2f86ba25f8da02401"
        },
        "date": 1780088402516,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 96132222,
            "range": "± 213526739",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3252421250,
            "range": "± 49974117",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22546745557,
            "range": "± 183765742",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "committer": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "distinct": true,
          "id": "fb7f93cc2f91e73d1b5bcc311ddb1a7012030dca",
          "message": "ci: consolidate workflows into rust.yml mirroring ddss\n\nReplace the separate ci.yml and bench.yml with a single rust.yml\ncopied from ddss for consistent experience across crates.\n\nAdaptations from the ddss template:\n- Remove submodules: recursive (dds-rs has no C++ vendored submodule)\n- MSRV entry: \"1.88\" instead of \"1.93\"\n- Clippy flags: --all-features -W clippy::nursery -W clippy::pedantic\n- Test and test-release add --all-features for feature-gated code paths",
          "timestamp": "2026-05-30T05:24:55+08:00",
          "tree_id": "a4579d1712d52166d478c54e78c8d4a8c96bd86c",
          "url": "https://github.com/jdh8/dds-rs/commit/fb7f93cc2f91e73d1b5bcc311ddb1a7012030dca"
        },
        "date": 1780090289141,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 101121200,
            "range": "± 227071314",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3307745832,
            "range": "± 38352676",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22727534920,
            "range": "± 301357006",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "committer": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "distinct": true,
          "id": "d01998728796c2f412ed70dba324361264f08530",
          "message": "Rename crate to pons-dds and migrate all public/internal references to pons_dds",
          "timestamp": "2026-05-30T06:47:32+08:00",
          "tree_id": "afbfbbe6c5307cbed5b3679044098ca381b6ab52",
          "url": "https://github.com/jdh8/pons-dds/commit/d01998728796c2f412ed70dba324361264f08530"
        },
        "date": 1780095318749,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 99282237,
            "range": "± 221567875",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3247927607,
            "range": "± 40283517",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22164916716,
            "range": "± 204350765",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "committer": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "distinct": true,
          "id": "9baad84a304b28227c076f51815ca091452a8fcc",
          "message": "Fix rustdoc private intra-doc links in public Solver/SearchStats docs",
          "timestamp": "2026-05-30T15:20:10+08:00",
          "tree_id": "055efb5031691b689687ea24ddf13ce75b3b6a76",
          "url": "https://github.com/jdh8/pons-dds/commit/9baad84a304b28227c076f51815ca091452a8fcc"
        },
        "date": 1780125994991,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 98434597,
            "range": "± 223894209",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3286997383,
            "range": "± 24112956",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22483072950,
            "range": "± 203979046",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}