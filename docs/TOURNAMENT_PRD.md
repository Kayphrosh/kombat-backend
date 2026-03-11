# Tournament & Pool Staking — Product Requirements Document

> **Version:** 1.0  
> **Date:** March 7, 2026  
> **Status:** Draft  
> **Author:** Engineering Team

---

## 1. Overview

### 1.1 Purpose

Enable users to stake USDC on tournament outcomes through a parimutuel betting system where odds are determined by the collective stakes of all participants.

### 1.2 Key Features

1. **Browse Tournaments** — View upcoming, live, and completed tournaments
2. **Pool Staking** — Stake on a team/player with dynamic odds
3. **P2P Tournament Wagers** — Challenge a friend on tournament outcomes
4. **Real-time Odds** — Live odds that update as stakes come in
5. **Stake History** — Track personal stakes and payouts

---

## 2. User Personas

| Persona           | Description                  | Goals                                     |
| ----------------- | ---------------------------- | ----------------------------------------- |
| **Casual Better** | New to betting, small stakes | Simple UX, understand odds easily         |
| **Active Staker** | Regular user, medium stakes  | Quick staking, track multiple tournaments |
| **Social Better** | Enjoys challenging friends   | P2P wagers, share results                 |

---

## 3. Screen Specifications

---

### 3.1 Tournaments List Page (`/tournaments`)

#### 3.1.1 Header Section

```
┌────────────────────────────────────────────────────────────┐
│  🏆 Tournaments                              [Filter ▼]    │
├────────────────────────────────────────────────────────────┤
│  [Upcoming]  [Live 🔴]  [Completed]  [My Stakes]           │
└────────────────────────────────────────────────────────────┘
```

**Components:**

- **Page Title:** "Tournaments" with trophy icon
- **Filter Dropdown:** Sort by sport/game type
- **Tab Navigation:**
  - `Upcoming` — Tournaments not yet started (default)
  - `Live` — Currently active (with live indicator)
  - `Completed` — Finished tournaments
  - `My Stakes` — User's active and past stakes

#### 3.1.2 Tournament Card Component

```
┌─────────────────────────────────────────────────────────────┐
│  [IMAGE]                                                    │
│                                                             │
│  🎮 FIFA 25 Championship                                    │
│  League of Legends • Esports                                │
│                                                             │
│  ┌─────────────────┐    ┌─────────────────┐                │
│  │ Team Alpha      │ vs │ Team Omega      │                │
│  │ 1.45x           │    │ 3.20x           │                │
│  │ $12,450 pool    │    │ $5,620 pool     │                │
│  └─────────────────┘    └─────────────────┘                │
│                                                             │
│  🕐 Starts in 2h 34m              Total Pool: $18,070      │
│                                                             │
│  [Stake Now]                    [Challenge Friend]          │
└─────────────────────────────────────────────────────────────┘
```

**Card States:**

| State              | Visual Treatment                                |
| ------------------ | ----------------------------------------------- |
| **Upcoming**       | Countdown timer, full color                     |
| **Live**           | Pulsing red dot, "LIVE" badge, staking locked   |
| **Completed**      | Greyed out, winner highlighted with ✓           |
| **User Has Stake** | Subtle highlight border, "Your stake: $X" badge |

**Card Data:**

- Tournament image/banner
- Tournament name
- Sport/game category
- Two outcome options with:
  - Team/player name
  - Current odds (calculated live)
  - Pool amount on this side
- Start time or status
- Total pool amount
- Action buttons

#### 3.1.3 Empty States

| Tab       | Empty State Message                                               |
| --------- | ----------------------------------------------------------------- |
| Upcoming  | "No upcoming tournaments. Check back soon!"                       |
| Live      | "No live tournaments right now."                                  |
| Completed | "No completed tournaments yet."                                   |
| My Stakes | "You haven't staked on any tournaments yet. [Browse Tournaments]" |

#### 3.1.4 Actions Available

| Action           | Trigger    | Behavior                                             |
| ---------------- | ---------- | ---------------------------------------------------- |
| View Details     | Tap card   | Navigate to tournament details                       |
| Stake Now        | Tap button | Navigate to tournament details with stake modal open |
| Challenge Friend | Tap button | Open P2P wager creation with tournament pre-selected |
| Filter           | Tap filter | Show filter bottom sheet                             |
| Switch Tab       | Tap tab    | Filter tournament list                               |
| Pull to Refresh  | Pull down  | Refresh tournament list                              |

---

### 3.2 Tournament Details Page (`/tournaments/:id`)

#### 3.2.1 Hero Section

```
┌─────────────────────────────────────────────────────────────┐
│  ← Back                                                     │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │                    [TOURNAMENT BANNER]                  ││
│  │                    FIFA 25 Championship                 ││
│  │                    League of Legends                    ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  Status: 🟢 UPCOMING          Starts: Mar 10, 2026 8:00 PM  │
│  Total Pool: $18,070          Stakers: 247                  │
└─────────────────────────────────────────────────────────────┘
```

**Hero Data:**

- Back navigation arrow
- Tournament banner image
- Tournament name
- Sport/game category
- Status badge (Upcoming/Live/Completed)
- Start time (countdown if < 24hrs)
- Total pool amount
- Number of unique stakers

#### 3.2.2 Outcome Selection Section

```
┌─────────────────────────────────────────────────────────────┐
│  Pick Your Winner                                           │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌──────────────────────────┐  ┌──────────────────────────┐ │
│  │      [TEAM LOGO]         │  │      [TEAM LOGO]         │ │
│  │                          │  │                          │ │
│  │      Team Alpha          │  │      Team Omega          │ │
│  │                          │  │                          │ │
│  │   ━━━━━━━━━━━━━━━━━━━━   │  │   ━━━━━━━━━━━━           │ │
│  │   69% of pool            │  │   31% of pool            │ │
│  │                          │  │                          │ │
│  │   Odds: 1.45x            │  │   Odds: 3.20x            │ │
│  │   Pool: $12,450          │  │   Pool: $5,620           │ │
│  │                          │  │                          │ │
│  │   ○ Select               │  │   ○ Select               │ │
│  └──────────────────────────┘  └──────────────────────────┘ │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Outcome Card Data:**

- Team/player logo or image
- Team/player name
- Visual progress bar (% of total pool)
- Percentage label
- Current odds multiplier
- Pool amount on this side
- Selection radio button

**Selection States:**

- **Unselected:** Default border, muted colors
- **Selected:** Highlighted border (brand color), filled radio
- **Disabled (Live/Completed):** Greyed out, no interaction

#### 3.2.3 Stake Input Section (Visible when outcome selected)

```
┌─────────────────────────────────────────────────────────────┐
│  Your Stake                                                 │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Amount (USDC)                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  $  [___100___]                              [MAX]      ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  Quick amounts:  [$10]  [$25]  [$50]  [$100]  [$500]        │
│                                                             │
│  Balance: $1,234.56 USDC                                    │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│  📊 Potential Returns                                       │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Your stake              $100.00                        ││
│  │  Current odds            3.20x                          ││
│  │  ─────────────────────────────────────                  ││
│  │  Min. payout if win      $320.00                        ││
│  │  Min. profit             $220.00  (+220%)               ││
│  │                                                         ││
│  │  ⚠️ Odds may change as more stakes come in              ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │              [  PLACE STAKE — $100  ]                   ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  By staking, you agree to the Terms of Service              │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Stake Input Components:**

- USDC amount input field
- MAX button (fill with wallet balance)
- Quick amount chips
- Wallet balance display
- Returns calculator:
  - Stake amount
  - Current odds
  - Minimum payout (stake × odds)
  - Minimum profit (payout - stake)
  - Percentage gain
- Warning about odds changes
- Primary CTA button
- Terms link

**Validation States:**

| State           | Visual Treatment             |
| --------------- | ---------------------------- |
| Empty           | Disabled CTA button          |
| Valid           | Active CTA button            |
| Exceeds balance | Red error text, disabled CTA |
| Below minimum   | "Minimum stake: $1" warning  |
| Network error   | Toast notification           |

#### 3.2.4 Your Stakes Section (If user has existing stakes)

```
┌─────────────────────────────────────────────────────────────┐
│  Your Stakes on This Tournament                             │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Team Omega  •  $50.00                                  ││
│  │  Placed: Mar 6, 2026 3:45 PM                            ││
│  │  Odds at stake: 4.10x  │  Current odds: 3.20x           ││
│  │  Potential payout: $160.00 → $150.40                    ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  Total staked: $50.00                                       │
│  Combined potential payout: $150.40                         │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Stake History Item:**

- Outcome picked
- Stake amount
- Timestamp
- Odds when staked vs current odds
- Payout change indicator (if odds shifted)

#### 3.2.5 Activity Feed Section

```
┌─────────────────────────────────────────────────────────────┐
│  Recent Activity                                   [See All]│
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  🟢  @CryptoKing staked $500 on Team Alpha        2m ago   │
│  🔵  @GameMaster staked $75 on Team Omega         5m ago   │
│  🟢  @LuckyBet staked $200 on Team Alpha         12m ago   │
│  🔵  @SolanaSam staked $150 on Team Omega        18m ago   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Activity Item:**

- Color indicator (matches outcome)
- Username (truncated wallet or display name)
- Stake amount
- Outcome picked
- Relative timestamp

#### 3.2.6 Challenge Friend Section

```
┌─────────────────────────────────────────────────────────────┐
│  Challenge a Friend                                         │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Want to go head-to-head on this tournament?                │
│  Create a P2P wager and challenge a friend directly.        │
│                                                             │
│  [Create P2P Wager on This Tournament →]                    │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

#### 3.2.7 Tournament States

**Upcoming State:**

- Full staking enabled
- Countdown to start
- Odds updating in real-time

**Live State:**

```
┌─────────────────────────────────────────────────────────────┐
│  🔴 LIVE — Staking Locked                                   │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  This tournament is in progress. Stakes are locked.         │
│  Final odds: Team Alpha 1.45x  •  Team Omega 3.20x          │
│                                                             │
│  [View Your Stakes]                                         │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Completed State — Winner:**

```
┌─────────────────────────────────────────────────────────────┐
│  🏆 Tournament Complete                                     │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Winner: Team Alpha ✓                                       │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  🎉 You Won!                                            ││
│  │                                                         ││
│  │  Your stake: $100.00  →  Payout: $145.00                ││
│  │  Profit: +$45.00 (+45%)                                 ││
│  │                                                         ││
│  │  [View Transaction]                                     ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Completed State — Lost:**

```
┌─────────────────────────────────────────────────────────────┐
│  ❌ You Lost                                                │
│                                                             │
│  Your stake: $100.00  →  Payout: $0.00                      │
│  Better luck next time!                                     │
│                                                             │
│  [Browse More Tournaments]                                  │
└─────────────────────────────────────────────────────────────┘
```

**Completed State — Refunded (One-sided pool):**

```
┌─────────────────────────────────────────────────────────────┐
│  ↩️ Stakes Refunded                                         │
│                                                             │
│  Not enough opposition for this tournament.                 │
│  Your stake of $100.00 has been refunded.                   │
│                                                             │
│  [View Transaction]                                         │
└─────────────────────────────────────────────────────────────┘
```

---

### 3.3 Stake Confirmation Modal

```
┌─────────────────────────────────────────────────────────────┐
│                    Confirm Your Stake                    ✕  │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│                    [TEAM LOGO]                              │
│                    Team Omega                               │
│                                                             │
│  Tournament      FIFA 25 Championship                       │
│  Your pick       Team Omega                                 │
│  Stake amount    $100.00 USDC                               │
│  Current odds    3.20x                                      │
│  Min. payout     $320.00 USDC                               │
│                                                             │
│  ─────────────────────────────────────────────────────────  │
│                                                             │
│  ⚠️ This action cannot be undone.                           │
│  Odds may change as more stakes are placed.                 │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │              [  CONFIRM STAKE  ]                        ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│                     [Cancel]                                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

---

### 3.4 My Stakes Page (`/stakes` or `/tournaments?tab=my-stakes`)

```
┌─────────────────────────────────────────────────────────────┐
│  My Stakes                                                  │
├─────────────────────────────────────────────────────────────┤
│  [Active]  [Won 🏆]  [Lost]  [Refunded]                     │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  FIFA 25 Championship            🟡 LIVE                ││
│  │  Team Omega • $100.00 staked                            ││
│  │  Current potential: $320.00                             ││
│  │  Staked: Mar 6, 2026                                    ││
│  │                                              [View →]   ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  NBA 2K Tournament               🟢 UPCOMING            ││
│  │  Lakers • $50.00 staked                                 ││
│  │  Current potential: $85.00                              ││
│  │  Staked: Mar 5, 2026                                    ││
│  │                                              [View →]   ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
├─────────────────────────────────────────────────────────────┤
│  Summary                                                    │
│  Active stakes: 2  •  Total at risk: $150.00                │
│  Potential return: $405.00                                  │
└─────────────────────────────────────────────────────────────┘
```

---

### 3.5 P2P Tournament Wager Creation

When user clicks "Challenge Friend" on a tournament:

```
┌─────────────────────────────────────────────────────────────┐
│  ← Create P2P Wager                                         │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Tournament                                                 │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  FIFA 25 Championship                          [✓]      ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  Your Pick                                                  │
│  ┌────────────────────┐  ┌────────────────────┐             │
│  │  ○ Team Alpha      │  │  ● Team Omega      │             │
│  └────────────────────┘  └────────────────────┘             │
│                                                             │
│  Wager Amount (USDC)                                        │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  $  [___100___]                                         ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  Challenge                                                  │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  [@] Enter wallet or username                           ││
│  └─────────────────────────────────────────────────────────┘│
│  Or leave empty for open challenge                          │
│                                                             │
│  ─────────────────────────────────────────────────────────  │
│                                                             │
│  Winner takes all: $200.00 (minus 1% fee)                   │
│  Auto-resolves when tournament ends                         │
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │              [  CREATE WAGER  ]                         ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Component Library

### 4.1 Odds Badge

```
┌───────────┐
│  3.20x    │  — Green if favorable (>2x)
└───────────┘     Yellow if moderate (1.5-2x)
                  Red if unfavorable (<1.5x)
```

### 4.2 Pool Progress Bar

```
Team Alpha  ━━━━━━━━━━━━━━━━━━━━░░░░░░░░░░  Team Omega
   69%                                         31%
```

### 4.3 Status Badges

- 🟢 `UPCOMING` — Green badge
- 🔴 `LIVE` — Red pulsing badge
- ⚫ `COMPLETED` — Grey badge
- 🏆 `WON` — Gold badge
- ❌ `LOST` — Red badge
- ↩️ `REFUNDED` — Blue badge

### 4.4 Stake Amount Input

- Currency symbol prefix
- Number input (decimals allowed)
- MAX button
- Quick amount chips below
- Validation error state

### 4.5 Returns Calculator Card

- Animated number transitions
- Green for profit
- Warning icon for odds disclaimer

---

## 5. User Flows

### 5.1 Place a Stake Flow

```
Tournaments List
      │
      ▼
Tournament Card Tap
      │
      ▼
Tournament Details Page
      │
      ▼
Select Outcome (Team A or B)
      │
      ▼
Enter Stake Amount
      │
      ▼
View Potential Returns
      │
      ▼
Tap "Place Stake"
      │
      ▼
Confirmation Modal
      │
      ▼
Wallet Signature Request
      │
      ▼
Success Toast + Update UI
```

### 5.2 View Stakes Flow

```
My Stakes Tab / Profile
      │
      ▼
Stakes List (filtered by status)
      │
      ▼
Stake Card Tap
      │
      ▼
Tournament Details (with your stake highlighted)
```

### 5.3 Tournament Resolution Flow

```
Tournament Ends
      │
      ▼
Status → LIVE → COMPLETED
      │
      ▼
Winner Declared (backend/admin)
      │
      ▼
Payouts Calculated
      │
      ▼
Push Notification Sent
      │
      ▼
User Views Result
      │
      ├── Won → See Payout Amount
      ├── Lost → See Loss
      └── Refunded → See Refund Reason
```

---

## 6. Real-Time Features

| Feature                  | Implementation                 |
| ------------------------ | ------------------------------ |
| Odds updates             | WebSocket or polling every 10s |
| New stakes activity      | WebSocket push                 |
| Tournament status change | Push notification + WebSocket  |
| Pool totals              | Live update on page            |

---

## 7. Error States & Edge Cases

| Scenario                   | User Message                               |
| -------------------------- | ------------------------------------------ |
| Insufficient balance       | "Not enough USDC. You need $X more."       |
| Tournament already started | "Staking closed. Tournament is live."      |
| Network error              | "Connection failed. Please try again."     |
| Transaction failed         | "Transaction failed. Your funds are safe." |
| One-sided pool (refund)    | "Not enough opposition. Stakes refunded."  |
| All picked same outcome    | "No winners to pay out. Stakes returned."  |

---

## 8. Accessibility Requirements

- Color contrast: 4.5:1 minimum for text
- Interactive elements: 44x44pt minimum touch target
- Screen reader labels for odds and percentages
- Keyboard navigation support
- Loading states with aria-live regions

---

## 9. Analytics Events

| Event                  | Properties                              |
| ---------------------- | --------------------------------------- |
| `tournament_viewed`    | tournament_id, source                   |
| `outcome_selected`     | tournament_id, outcome_id               |
| `stake_amount_entered` | tournament_id, amount                   |
| `stake_placed`         | tournament_id, outcome_id, amount, odds |
| `stake_confirmed`      | tournament_id, tx_hash                  |
| `stake_failed`         | tournament_id, error_type               |
| `p2p_wager_created`    | tournament_id, amount                   |

---

## 10. Open Questions

1. **Minimum stake amount?** — Suggest $1 USDC
2. **Maximum stake per user per tournament?** — Unlimited or cap?
3. **Can users stake on multiple outcomes?** — Recommend: No (pick one side)
4. **Staking deadline?** — Lock at tournament start or X minutes before?
5. **Platform fee?** — Suggest 2-5% of winning pool
6. **Odds format?** — Decimal (3.20x) vs American (+220)?

---

## 11. Future Enhancements

- **Leaderboard** — Top stakers by profit
- **Social sharing** — Share stakes on Twitter/X
- **Notifications** — Odds movement alerts
- **Parlay bets** — Combine multiple tournaments
- **Liquidity pools** — Seed initial odds
