## Index

- docs-written-from-code: how these project documents were produced
- review-freshness: which commit the call-graph snapshot is compared to

## docs-written-from-code

These project documents were written in one pass from the README, Cargo
manifests, CI and existing `docs/`, because the project's Incord docs gate
refused source edits without them. They are deliberately short and point to the
detailed docs. Cost to undo: delete the files (and the gate will block code
edits again). Owners should correct anything that does not match intent.

## review-freshness

`review::impact` compares `plan.graph_commit` to the BASE commit, not the head.
`docs/REVIEW.md` documents `graph_commit` as `exact-baseline-sha` and the unit
test uses `"base"`. A first review pass called this reversed; it is the
documented design. New files are not in the graph and force a full check run.
