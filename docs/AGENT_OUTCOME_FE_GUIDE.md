# Frontend Implementation Guide — Agent Outcome Resolution

This guide covers every frontend-facing feature of the automated outcome
resolution system: how match results flow from PandaScore (or organizers) into
**outcome proposals**, how they get auto-verified or queued for review, and the
admin screens needed to review and settle them.

The FE dev does **not** need to build the poller or webhooks — those are
server-side and run automatically. The FE work is:

1. **Admin review queue** — list proposals, view evidence, approve/reject/dispute
2. **Agent run audit view** — list/inspect agent runs and their verification status
3. **Match result badges** — surface `verification_status` / `result_status` on match pages
4. **Evidence viewer** — render the Walrus-stored evidence blob

---

## 1. Conventions

### Base URL
All endpoints are under the API host, e.g. `https://<your-api>.railway.app`.

### Response envelope
Every endpoint returns this shape:

```ts
interface ApiResponse<T> {
  success: boolean;
  data: T | null;
  error: string | null;
}
```

Always check `success` before reading `data`.

### Auth headers
| Audience | Header | Value |
|---|---|---|
| Admin endpoints | `x-admin-token` | the admin token (server `AUTH_ADMIN_TOKEN`) |
| User/wallet endpoints | `Authorization` | `Bearer <jwt>` |

> Admin token is a shared secret — only use it from an authenticated admin
> context (an internal admin dashboard), never ship it to public clients.

### Status vocabulary
A proposal / agent run carries a **verification_status**:

| Value | Meaning | FE treatment |
|---|---|---|
| `auto_verified` | PandaScore confirmed the winner; high confidence | Green badge "Auto-verified" |
| `pending_review` | Needs a human decision (disagreement, low confidence, no PandaScore data) | Amber badge "Needs review" + show in queue |

A proposal's lifecycle **status** column takes these values:

| `status` | Set when |
|---|---|
| `pending` | Created directly by an organizer (`POST /api/tournaments/:id/outcome-proposals`) and not yet verified |
| `pending_review` | Auto-pipeline (agent/webhook/poller) needs a human decision |
| `auto_verified` | Auto-pipeline confirmed via PandaScore + confidence |
| `approved` / `rejected` / `disputed` | After an admin review (3.4) |

A match has a **result_status**: `pending`, `proposed`, `approved`, `rejected`,
`disputed`, and a **verification_status** string like `agent_auto_verified`,
`pandascore_poller_pending_review`, etc. (format is `<source>_<status>`).

---

## 2. Data models (TypeScript)

```ts
interface OutcomeProposal {
  id: string;
  match_id: string;
  proposed_winner_opponent_id: string | null;
  proposed_winner_name: string | null;
  source: string;            // "agent" | "pandascore_poller" | "organizer_webhook" | "pandascore_webhook" | "organizer"
  proposer_wallet: string | null;
  confidence: string | null; // decimal as string, e.g. "1.0000"
  status: string;            // pending | auto_verified | approved | rejected | disputed
  evidence_blob_id: string | null;
  evidence_url: string | null;   // aggregator URL to fetch the blob directly
  evidence_summary: string | null;
  raw_data: unknown;         // the full evidence JSON
  created_at: string;        // ISO8601
  reviewed_at: string | null;
  reviewer_wallet: string | null;
}

interface AgentRun {
  id: string;
  match_id: string | null;
  agent_name: string;
  agent_id: string | null;
  status: string;            // "completed" | "queued" | ...
  watch_sources: unknown;
  evidence_blob_id: string | null;
  evidence_url: string | null;
  outcome_proposal_id: string | null;
  proposed_winner_opponent_id: string | null;
  proposed_winner_name: string | null;
  confidence: string | null;
  summary: string | null;
  error: string | null;
  verification_status: string | null;  // auto_verified | pending_review
  verification_note: string | null;    // human-readable reason
  raw_output: unknown;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
  updated_at: string;
}

interface WalrusArtifact {
  id: string;
  blob_id: string;
  object_id: string | null;
  artifact_type: string;     // "agent_evidence" | "poller_evidence" | "webhook_evidence"
  owner_wallet: string | null;
  match_id: string | null;
  outcome_proposal_id: string | null;
  content_type: string;
  size_bytes: number;
  aggregator_url: string | null;
  metadata: unknown;
  created_at: string;
}
```

---

## 3. Endpoints

### 3.1 Admin — list outcome proposals (the review queue)

```
GET /api/admin/outcome-proposals
Header: x-admin-token: <token>
Query (all optional):
  status   — filter: pending | auto_verified | approved | rejected | disputed
  source   — filter: agent | pandascore_poller | organizer_webhook | ...
  match_id — filter by match UUID
  limit    — default 50, max 200
  offset   — default 0
```

Returns `ApiResponse<OutcomeProposal[]>`.

**Build the review queue** by fetching `?status=pending_review` — these are the
proposals from the auto-pipeline awaiting an admin decision. Also fetch
`?status=pending` if you want to surface organizer-created proposals that
haven't been verified.

> Tip: show tabs — **Needs review** (`status=pending_review`),
> **Auto-verified** (`status=auto_verified`), and a **History** tab for
> approved/rejected/disputed.

### 3.2 Admin — get a single proposal

```
GET /api/admin/outcome-proposals/:id
Header: x-admin-token: <token>
```

Returns `ApiResponse<OutcomeProposal>`.

### 3.3 List proposals for a match (no admin token required)

```
GET /api/tournaments/:matchId/outcome-proposals
```

Returns `ApiResponse<OutcomeProposal[]>` — useful on a public match detail page
to show "result proposed, pending verification."

### 3.4 Admin — review (approve / reject / dispute) a proposal

```
POST /api/outcome-proposals/:id/review
Header: x-admin-token: <token>
Body:
{
  "decision": "approve" | "reject" | "dispute",
  "reviewer_wallet": "0x..."   // optional, for audit trail
}
```

Returns `ApiResponse<OutcomeProposal>` (updated).

**Important side effect:** when `decision = "approve"` and the proposal has a
`proposed_winner_opponent_id`, the backend **resolves the match and triggers
payout calculation**. So the Approve button is the settlement action — confirm
with the admin before calling it.

### 3.5 Admin — list agent runs (audit log)

```
GET /api/admin/agent-runs
Header: x-admin-token: <token>
Query (all optional):
  status      — e.g. completed
  agent_name  — e.g. kombat-outcome-agent
  agent_id    — per-agent identity (for accuracy auditing)
  match_id    — by match UUID
  limit / offset
```

Returns `ApiResponse<AgentRun[]>`.

### 3.6 Admin — get a single agent run

```
GET /api/admin/agent-runs/:id
Header: x-admin-token: <token>
```

Returns `ApiResponse<AgentRun>`.

### 3.7 Walrus — fetch evidence

You have two ways to render the evidence blob:

**Option A — use the `evidence_url` directly** (simplest). Both
`OutcomeProposal.evidence_url` and `AgentRun.evidence_url` are full aggregator
URLs. Just `fetch()` it and render the JSON.

**Option B — resolve a blob id to a URL:**
```
GET /api/walrus/blobs/:blobId/url
```
Returns `ApiResponse<{ blob_id: string; url: string }>`.

**Walrus config (to know if evidence storage is even on):**
```
GET /api/walrus/config
```
Returns `ApiResponse<{ enabled, configured, network, aggregator_url, max_upload_bytes }>`.

---

## 4. Screens to build

### A. Admin Review Queue
- Table of proposals from `GET /api/admin/outcome-proposals?status=pending`
- Columns: match name, proposed winner, source, confidence, verification badge, created_at
- Row click → detail drawer

### B. Proposal Detail Drawer
- Header: match, proposed winner, source, confidence
- **Verification banner**: green (auto_verified) or amber (pending_review).
  Pull the human-readable reason from the linked agent run's
  `verification_note` (fetch `GET /api/admin/agent-runs?match_id=<id>` and match
  on `outcome_proposal_id`).
- **Evidence panel**: fetch `evidence_url` and pretty-print the JSON. Show
  `evidence_summary` prominently.
- **Action buttons**: Approve (settles + pays out — needs confirm modal),
  Reject, Dispute → `POST /api/outcome-proposals/:id/review`.

### C. Agent Run Audit
- Table from `GET /api/admin/agent-runs`
- Filter by `agent_id` to compare agent accuracy (compare `proposed_winner_name`
  vs. the eventual approved winner over time)
- Show `verification_status`, `verification_note`, `confidence`, `summary`,
  `error`.

### D. Public Match Detail (verification badge)
- On a match page, call `GET /api/tournaments/:matchId/outcome-proposals`
- If a proposal exists, show a status chip based on `status`:
  - `auto_verified` / `approved` → "Result confirmed: {winner}"
  - `pending` / `pending_review` → "Result proposed — under review"
  - `rejected` → "Result rejected"
  - `disputed` → "Result disputed"

---

## 5. Reference: how a result becomes a proposal (context only)

You don't build this, but it helps to understand the states:

1. A match finishes. One of three sources creates a proposal:
   - **PandaScore poller** (runs every ~2 min server-side) — source
     `pandascore_poller`, confidence `1.0`, almost always `auto_verified`.
   - **Organizer webhook** (`POST /api/webhooks/match-result`) — source
     `organizer_webhook`.
   - **External agent** (`POST /api/agents/outcome-proposals`) — source `agent`.
2. The backend uploads the evidence to Walrus and cross-checks against
   PandaScore.
3. Result: `auto_verified` (PandaScore agrees + confidence high enough) or
   `pending_review`.
4. Admin reviews `pending_review` items; **approving settles the match**.

> Auto-verified proposals are NOT auto-settled — they still appear in the admin
> dashboard. If you want auto-verified results to settle without a click, that's
> a backend change; ask the backend dev. Today, **approval is always a human
> action via 3.4**.

---

## 6. Quick fetch helper

```ts
const API = import.meta.env.VITE_API_URL;
const ADMIN_TOKEN = /* from secure admin session, never hardcode */;

async function adminGet<T>(path: string): Promise<T> {
  const res = await fetch(`${API}${path}`, {
    headers: { "x-admin-token": ADMIN_TOKEN },
  });
  const body = (await res.json()) as ApiResponse<T>;
  if (!body.success || body.data == null) throw new Error(body.error ?? "Request failed");
  return body.data;
}

async function reviewProposal(id: string, decision: "approve" | "reject" | "dispute", reviewerWallet?: string) {
  const res = await fetch(`${API}/api/outcome-proposals/${id}/review`, {
    method: "POST",
    headers: { "x-admin-token": ADMIN_TOKEN, "Content-Type": "application/json" },
    body: JSON.stringify({ decision, reviewer_wallet: reviewerWallet }),
  });
  const body = await res.json();
  if (!body.success) throw new Error(body.error);
  return body.data;
}
```
