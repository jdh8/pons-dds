window.BENCHMARK_DATA = {
  "lastUpdate": 1783460118465,
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
          "id": "df48e50f76b80e469bbd0f0b818204d895f3fb1c",
          "message": "Add Apache-2.0 LICENSE file for 0.1.0 release\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-05-30T16:13:19+08:00",
          "tree_id": "306132dc1945b518e71ce92490a8426fa34e38b3",
          "url": "https://github.com/jdh8/pons-dds/commit/df48e50f76b80e469bbd0f0b818204d895f3fb1c"
        },
        "date": 1780129350490,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 102011595,
            "range": "± 226096074",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3287412562,
            "range": "± 45163163",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22657737425,
            "range": "± 218328942",
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
          "id": "9a1282035e907dafd01a6d9223c5f4b008bec003",
          "message": "Rewrite 0.1.0 changelog to describe purpose and API\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-05-30T16:32:58+08:00",
          "tree_id": "2c2d38244f616d65e6728ca8ccd647f81e1cd5c1",
          "url": "https://github.com/jdh8/pons-dds/commit/9a1282035e907dafd01a6d9223c5f4b008bec003"
        },
        "date": 1780130365397,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 100855893,
            "range": "± 230697038",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3272689410,
            "range": "± 25158556",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22595780223,
            "range": "± 436527852",
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
          "id": "6acf889c3a4c5a966f88c95a0eca81387e26effc",
          "message": "Credit DDS/ddss lineage in README acknowledgements",
          "timestamp": "2026-05-30T16:40:43+08:00",
          "tree_id": "8256c8e489502035b6701946159f12763c6a1be1",
          "url": "https://github.com/jdh8/pons-dds/commit/6acf889c3a4c5a966f88c95a0eca81387e26effc"
        },
        "date": 1780130783878,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 81554788,
            "range": "± 173033555",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2610336333,
            "range": "± 18886544",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 17940568083,
            "range": "± 415863506",
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
          "id": "c8bc2f09176f7391c18b4b32d432eeab966a6802",
          "message": "Update link to CI",
          "timestamp": "2026-05-30T16:46:34+08:00",
          "tree_id": "745fa53e651edad3bd970ac29c289b410e75b435",
          "url": "https://github.com/jdh8/pons-dds/commit/c8bc2f09176f7391c18b4b32d432eeab966a6802"
        },
        "date": 1780131211816,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 107559558,
            "range": "± 225619336",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3678486317,
            "range": "± 17798637",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 25362750632,
            "range": "± 96152302",
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
          "id": "c1bc3a30f93e12b8bbee54953293b7f2663af2a8",
          "message": "Use README.md as crate docs via include_str!\n\nAlign pons-dds with the other workspace crates (pons, ddss,\ncontract-bridge), whose lib.rs starts with\n`#![doc = include_str!(\"../README.md\")]` for a single source of truth.\n\n- Fold the inline v0.1-scope docs into a new README \"Scope\" section.\n- Merge the per-module / ABsearch.cpp algorithm-reference note into the\n  existing Acknowledgements paragraph, dropping the broken relative link.\n- Turn the README usage blocks into real, passing doctests (use\n  `.unwrap()`, define `deals`, add hidden setup for the second block).\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-05-30T16:58:18+08:00",
          "tree_id": "d8d1d182b6c6f8aaa36aa93ba9f6d49596284917",
          "url": "https://github.com/jdh8/pons-dds/commit/c1bc3a30f93e12b8bbee54953293b7f2663af2a8"
        },
        "date": 1780131835771,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 80621566,
            "range": "± 184513250",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2622309720,
            "range": "± 14331940",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 17823525069,
            "range": "± 66831310",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "noreply@anthropic.com",
            "name": "Claude",
            "username": "claude"
          },
          "committer": {
            "email": "chen.pang.he@jdh8.org",
            "name": "Chen-Pang He",
            "username": "jdh8"
          },
          "distinct": true,
          "id": "c290b7e23604fe2ac435bd8658631b881a82ce71",
          "message": "Fix LICENSE link branch: master -> main\n\nThe default branch was renamed to main.",
          "timestamp": "2026-05-30T19:08:08+08:00",
          "tree_id": "4593dee7342e763510930645621bba754beb32f2",
          "url": "https://github.com/jdh8/pons-dds/commit/c290b7e23604fe2ac435bd8658631b881a82ce71"
        },
        "date": 1780139662504,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 95853678,
            "range": "± 205582242",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3261516517,
            "range": "± 40368875",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 22009940771,
            "range": "± 362999408",
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
          "id": "85e3451597a3e4faf85271f2ac91bcfcd9ebfb5f",
          "message": "Add parallel TT-budget sweep and load-balance diagnostics\n\ntt_sweep now sweeps the TT budget warm across the whole pool instead of\nsingle-threaded; new par_balance reports the makespan tail ratio and per-strain\nsolve-time distribution. Both help tune task dispatch and TT sizing on a given\nmachine.\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-05-31T02:50:20+08:00",
          "tree_id": "29740693a98b8eb5c1e93b36e3e6297b7c4dde0a",
          "url": "https://github.com/jdh8/pons-dds/commit/85e3451597a3e4faf85271f2ac91bcfcd9ebfb5f"
        },
        "date": 1780167411866,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 101102350,
            "range": "± 206051167",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3487246020,
            "range": "± 108286429",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 23291317277,
            "range": "± 732796410",
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
          "id": "5f2a6f8b75473b804e9fae7d50602b3b8fcda18c",
          "message": "Release 0.1.1\n\nBug-fix and additive release over 0.1.0.\n\n- Fix: parallel batch solving (solve_deals) could overflow Rayon's\n  default worker/calling-thread stacks on larger batches, and overflowed\n  readily on Windows' 1 MiB stacks. The deep search now runs only on the\n  solver pool's large-stack workers (regression test\n  solve_deals_safe_on_small_stack).\n- Add: solve_deals_with_memory for an explicit per-thread\n  transposition-table budget; examples/par_balance load-balance diagnostic.\n\nPublic API is additive only, so this is semver-compatible (0.1.0 -> 0.1.1).\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-05-31T02:56:42+08:00",
          "tree_id": "caee329e28ebf9ae64fa9eaf8cea94fdc7a7d155",
          "url": "https://github.com/jdh8/pons-dds/commit/5f2a6f8b75473b804e9fae7d50602b3b8fcda18c"
        },
        "date": 1780167843994,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 107562649,
            "range": "± 208200402",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3812597733,
            "range": "± 74191447",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 26283217919,
            "range": "± 875622584",
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
          "id": "ec2e17ef104bf91cf3aee733c9ff66028365ba09",
          "message": "docs(readme): move GitHub badge to third position",
          "timestamp": "2026-06-01T07:46:23+08:00",
          "tree_id": "17a7836eed3edaa34da246956a0e759818265bc6",
          "url": "https://github.com/jdh8/pons-dds/commit/ec2e17ef104bf91cf3aee733c9ff66028365ba09"
        },
        "date": 1780271584478,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 84326100,
            "range": "± 171759315",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2717563911,
            "range": "± 61068049",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 20629846346,
            "range": "± 526568146",
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
          "id": "f9072fde5400ebc17c9fd10a42faa95e7b0ec8a4",
          "message": "Release 0.1.2\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-05T21:05:30+08:00",
          "tree_id": "6bdc2fd7eef06408db9f97b5c6367e94197a3688",
          "url": "https://github.com/jdh8/pons-dds/commit/f9072fde5400ebc17c9fd10a42faa95e7b0ec8a4"
        },
        "date": 1783257138994,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 94562058,
            "range": "± 192980347",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 3395301805,
            "range": "± 64885198",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 24041080515,
            "range": "± 728278829",
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
          "id": "65d2c1d4e609ca017998bc5283cfe1186deacb75",
          "message": "docs(changelog): record the optimization pass\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-06T08:07:02+08:00",
          "tree_id": "670b19c083d148d089b8de35b4457ee4762ad59e",
          "url": "https://github.com/jdh8/pons-dds/commit/65d2c1d4e609ca017998bc5283cfe1186deacb75"
        },
        "date": 1783316526268,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 78853689,
            "range": "± 166953700",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2863437692,
            "range": "± 62913951",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 20382128174,
            "range": "± 724267473",
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
          "id": "9ebcb21101b8430725e69eac4024636220a257e1",
          "message": "perf: retain the TT page pool across per-strain resets\n\nreset() truncated the page Vec to one page while the allocator handed\nblocks out of pages.len()-1, so every strain re-malloc'd and zeroed\nfresh 6.2 MiB pages. Track next_page/next_slot explicitly and keep\npages_default pages alive across reset (vendor ResetMemory parity),\nreusing the slabs and only rewinding each block's counters on hand-out.\n\nCuts page allocations ~4210 to ~54 per 200 sequential deals (~26 GB to\n~0.3 GB of malloc+memset). Single-thread bisection_stats ~3% faster; a\n1000-deal parallel batch ~10% faster (p<0.05), narrowing the same-run\ngap to ddss from 1.26x to 1.13x with ddss itself flat (p=0.33) so\nthermal drift is ruled out. Bit-for-bit unchanged (10k-deal soak).\nNet -13 LOC.\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-07-06T14:36:06+08:00",
          "tree_id": "a6c60c2e57b01a6bd309ad353354ea24b5819932",
          "url": "https://github.com/jdh8/pons-dds/commit/9ebcb21101b8430725e69eac4024636220a257e1"
        },
        "date": 1783321145597,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 87304932,
            "range": "± 183898435",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2971305543,
            "range": "± 41404643",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 21476058536,
            "range": "± 825753290",
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
          "id": "234c0f6ca9bae017c901a5cd784cadd3334d236e",
          "message": "docs: cover the new features in benches, README, and CHANGELOG\n\nAdd the solve_boards/{32,200} and analyse_plays_32 bench cases,\nmirroring the dds-bridge crate's layout for the gh-pages dashboard, and\ndrop the note about them being omitted. The README gains usage examples\nfor solve_board, analyse_play, and calculate_par plus a ddss→pons-dds\nmigration table in the Scope section; the CHANGELOG records the new API\nsurface, the two breaking changes, and the documented divergences from\nthe FFI reference.\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-07T19:47:09+08:00",
          "tree_id": "12621ed8538db3dd5d0125975c405959ea6c4cc6",
          "url": "https://github.com/jdh8/pons-dds/commit/234c0f6ca9bae017c901a5cd784cadd3334d236e"
        },
        "date": 1783425477149,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 78722795,
            "range": "± 168945408",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2807166110,
            "range": "± 56819344",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 20721271356,
            "range": "± 727223808",
            "unit": "ns/iter"
          },
          {
            "name": "solve_boards/32",
            "value": 298712022,
            "range": "± 2348208",
            "unit": "ns/iter"
          },
          {
            "name": "solve_boards/200",
            "value": 1559981522,
            "range": "± 31417322",
            "unit": "ns/iter"
          },
          {
            "name": "analyse_plays_32",
            "value": 292547881,
            "range": "± 4507376",
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
          "id": "4c8e540a140e79cf17a10e46c48691f64ba2827f",
          "message": "release: cut 0.2.0\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-07-08T05:26:29+08:00",
          "tree_id": "17821e15360d3dd03541fe23d10ba79c4b52f796",
          "url": "https://github.com/jdh8/pons-dds/commit/4c8e540a140e79cf17a10e46c48691f64ba2827f"
        },
        "date": 1783460117681,
        "tool": "cargo",
        "benches": [
          {
            "name": "solve_deal",
            "value": 77904022,
            "range": "± 167277713",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/32",
            "value": 2817831699,
            "range": "± 45501178",
            "unit": "ns/iter"
          },
          {
            "name": "solve_deals/200",
            "value": 20057230462,
            "range": "± 642117319",
            "unit": "ns/iter"
          },
          {
            "name": "solve_boards/32",
            "value": 302515867,
            "range": "± 2005831",
            "unit": "ns/iter"
          },
          {
            "name": "solve_boards/200",
            "value": 1572010286,
            "range": "± 30678995",
            "unit": "ns/iter"
          },
          {
            "name": "analyse_plays_32",
            "value": 301476674,
            "range": "± 5879073",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}