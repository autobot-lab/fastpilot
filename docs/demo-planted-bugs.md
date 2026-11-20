# Technique demo pages: the planted bugs

`fastpilot demo` writes three pages for practising test design techniques. Each page states its rule
on screen (black-box techniques need a specification) and breaks it in **one** place. This file is
the answer key for checking that the skills find them. It lives only in the repository: the demo
folder gets the pages, not this file, so an AI testing the pages cannot read the answers.

| Page | Technique it is for | The rule on the page | The planted bug | Found by | Usually missed by |
|---|---|---|---|---|---|
| `tickets.html` (Event tickets) | Equivalence partitioning, boundary value analysis | Age 18 to 65; Tickets 1 to 10 per order, whole numbers | **10 tickets is refused** ("Tickets must be 1 to 10"); 1–9 are accepted | BVA: the maximum (10) | EP with one value per class (e.g. 5): passes |
| `shipping.html` (Shipping cost) | Decision table | Standard $5.00, free for members on orders of $50.00 or more; Express $15.00, members $10.00 | **Member + Express + order of $50.00 or more costs $0.00** (should be $10.00); the other 7 combinations are right | Decision table: the rule member = yes, ≥ $50 = yes, express = yes | Checking a few likely cases (member standard, guest express) |
| `orders.html` (Order #1001) | State transition | Created → Paid → Shipped → Delivered; cancel only while Created or Paid | **A Shipped order can be cancelled** ("Cancel order" stays enabled and works); every other forbidden move is disabled | State transition: the forbidden event cancel from Shipped | Testing only the allowed path |

## Expected results per page

### tickets.html
| Age | Tickets | Expected | What the page does |
|---|---|---|---|
| 17 | 2 | Refused: age | ✓ refused |
| 18 | 2 | Reserved | ✓ |
| 65 | 2 | Reserved | ✓ |
| 66 | 2 | Refused: age | ✓ refused |
| 30 | 0 | Refused: tickets | ✓ refused |
| 30 | 1 | Reserved | ✓ |
| 30 | 9 | Reserved | ✓ |
| 30 | **10** | **Reserved** | **✗ refused (bug)** |
| 30 | 11 | Refused: tickets | ✓ refused |
| 30 | `abc` / `2.5` | Refused: whole number | ✓ refused |

### shipping.html
| Member | Total ≥ $50 | Express | Expected | What the page does |
|---|---|---|---|---|
| Y | Y | N | $0.00 | ✓ |
| Y | N | N | $5.00 | ✓ |
| N | Y | N | $5.00 | ✓ |
| N | N | N | $5.00 | ✓ |
| **Y** | **Y** | **Y** | **$10.00** | **✗ $0.00 (bug)** |
| Y | N | Y | $10.00 | ✓ |
| N | Y | Y | $15.00 | ✓ |
| N | N | Y | $15.00 | ✓ |

Boundary of the total: $49.99 is under, $50.00 is "$50 or more".

### orders.html
| From \ event | Mark as paid | Ship | Mark as delivered | Cancel order |
|---|---|---|---|---|
| Created | Paid | not allowed | not allowed | Cancelled |
| Paid | not allowed | Shipped | not allowed | Cancelled |
| Shipped | not allowed | not allowed | Delivered | **not allowed → bug: Cancelled** |
| Delivered | not allowed | not allowed | not allowed | not allowed |
| Cancelled | not allowed | not allowed | not allowed | not allowed |

"Not allowed" shows as a disabled button. "Start a new order" goes back to Created.

`bin/fastpilot/tests/technique_demo.rs` checks these tables against the pages, so a change that
fixes or moves a bug by accident fails the build. Fixing a bug on purpose means changing this file
and that test together.
