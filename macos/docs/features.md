# Functions of Carbide Silica

The app can do what Carbide's terminal app does. [`docs/reference.md`](../../docs/reference.md) is the reference.
The app shows the same data as pictures where a picture is clearer, and keeps the exact
figures and the raw JSON one step away.

**Places**

- Dashboard: a status line (height against the longest chain, age of the last block, node
  and API state, peers), the chain pulse (bursts of blocks on a time axis, with the current
  wait set among a day of recorded waits), recent blocks, and recent confirmed
  transactions with their tags. Stream blocks keeps the newest block in view.
- Mempool: a waiting list on a time record. One sentence says what waits and how the
  wait since the last block compares with recent waits. The last 15 minutes are one
  axis with each block on it and a line for each transaction from its arrival to its
  block. Each waiting transaction is a row with its tags, the sentence of its
  statement, its wait, fee, size, expiry and waiting parents, in order of arrival. One
  bar shows the part of a block that the queue fills. Below, the transactions that the
  last 30 blocks took, with the wait of each one that the app saw. The second view is
  the terminal's list with arrival, fee, size, entry height, unconfirmed parents and
  tags. `../design/mempool-research.html` has the study behind this form.
- Charts: block rate, mining and staking difficulty with MA100 lines, the staked share
  over 100 and 1,000 blocks, and the fee pool, as five panels on one time axis with one
  cursor through all of them. The block rate has process limits and the staked share has
  the limits of chance, so a signal is distinct from noise. Spans are 2 hours, 6 hours,
  1 day or 7 days of header time. You can page to older and newer ranges, go to a block or
  a date, and select a block to see its exact figures. `../design/charts-research.html` has
  the reasons for the form.
- Future block estimates from Carbide's calibrated rate, with the calibration shown.
- Currencies: Carbide's filters. Each option flag is Any, Show or Hide, and there are
  launch-status filters. The default view is liquidity pools without failed launches.
- Marketplace: the open offers as a catalogue. An answer line (offers, sellers, the share
  of the largest seller, lowest and median price), the prices that many offers share as
  bars, one curve of the offers available at or below each price, and a list with a name
  search and filters that show their counts: kind of name, length, price, seller and
  listing period. Carbide's four sort orders and the links to each offer's identity,
  currencies, funding transaction and addresses stay. `../design/market-research.html` has
  the reasons for the form.
- Bookmarks, in Carbide's own bookmark file, including notarization views with their
  perspective, manual entry, and Carbide's optional start-up population.
- Node and peers: synchronization progress, validation speed, peers and bans in one
  list, network totals, a peer page with the node log and raw JSON, a map of peers, and
  side branches. The peer list starts with one sentence and a count of the peers by
  network operator and by country. Each row has its measures on scales that all rows
  share: the response time as a dot with the range of the session, and the data received
  as a bar. The map opens on the area that holds all placed peers, joins near peers into
  one mark, and is drawn by the app from the coast points of the terminal's map: it
  makes no request to a map service. `../design/peers-research.html` has the study behind
  this form.
- Sources: each node and public API with its state (Disabled, Enabled, Monitored), its
  traffic and its recorded activity.

**Objects**

- Search by block height or hash, transaction ID, date, address, identity (`name@`) or
  currency (`name@@`). Search also accepts `currencies`, and `estimate:` with a height or
  a date. A future height or date opens an estimate.
- Blocks: header, chain status, the tags of all transactions, stored child branches, and
  a link to the active block when the block is on a side branch.
- Transactions: Carbide's overview (status, totals, each fee variant, shielded and
  registration lines), a statement, inputs and outputs with Carbide's annotations and
  maturity badges, and a byte view for unrecognised output data. The statement is one
  sentence, then one row for each party with its net change in each currency: a hollow
  bar for an amount given, a solid bar for an amount received. The reserve deposits of a
  currency are one party. An amount that only passes through is a figure, outputs
  without value are named marks, and a reserve transfer shows what it asks for. A row
  marks its inputs and outputs in the lists. `../design/txflow-research.html` has the study
  behind this form.
- Transfer trace: Carbide's DeFi trail with one verdict, then a receipt that reads from
  the amount sent to the amount received with the fee and the rate of the batch, the
  stages on an axis of blocks so that the wait for an export is a length, and the place
  of the transfer in its export batch. Below are live progress, leg budgets, execution
  rates, a comparison with the previous pool notarization, and searches of other
  destination windows. `../design/trace-research.html` has the study behind this form.
- Witnesses and protocol references, forward and reverse, with Carbide's status for each.
- Identities at each version: the history as lanes for the keys, the revoke authority,
  the recover authority and the lock, with one step for each version and a time line of
  the true blocks. The control of the viewed version is one sentence, a table of the four
  powers, and the outcome of a lost key and of a stolen key in words.
  `../design/identity-research.html` has the study behind this form. Registration evidence, funding candidates, the referral
  chain, verified registration payments, tokenized control, and historical reverse
  requests.
- Baskets in one unit: the Currency page of a basket has a control that selects one of
  its reserves as the unit. The reserves table then shows the value of each reserve and
  what one of it is worth in that unit. The lines over time show the basket and each
  reserve in that unit on one scale that starts at 100, and the last conversion batch is
  a statement with one bar for each reserve that moved. The values are the pool's own
  rates. `../design/currency-research.html` has the study behind this form.
- Currencies: definition, reserves, supply and prices over time, the state at an exact
  block, imports and exports in 1,000-block windows, the conversion batch, and the
  notarization browser with checks of each foreign proof root. The browser gives the
  state of the link first: the wait since the last confirmed notarization against the
  earlier intervals, on one scale. The entries that the node has are a graph: the best
  chain is the straight line, and a side branch is a second line.
  `../design/notarization-research.html` has the study behind this form.
- Addresses: balances, first and latest activity, the identities that use the address as
  a primary address, and transactions as input and output tiles.
- Raw JSON for each object as a tree with links, names, dates, copy and save.

**Sources and stored data**

- Each page shows the source of its data, and shows if the data is live or stored.
- The app records the answers for the pages you open in Carbide's cache. When no source
  answers, a page that you opened before shows the stored observation, marked as stored.
- Charts read blocks without adding them to the cache. A chart load does not make the
  cache grow.
- When the node cannot answer a page or a search, the app asks before it sends the
  request to a disabled public API. Automatic requests never ask.
- Block history and the child branches of a block come from stored observations. A
  background check keeps the stored chain current for the connected node and for each
  monitored node.

**Display and keyboard**

- View > Show Names for IDs, Show Dates for Heights and Show Times in UTC. The same
  settings are in Settings.
- The keys of the terminal work in the main window when no text field is in use.
  Each page has a cursor: j and k (or the arrow keys) move it through the rows and the
  links of the page, Return opens, Esc goes back. One letter goes to a place (d m o c),
  n and p step to the next and previous block in place, and F8 and F4 start on the
  output under the cursor. `../design/power-user-review.html` has the review behind this.
- Command-K opens the command list: each command of the app and each card of the open
  page, with its key. Tab shows the raw JSON of a page, where the cursor moves through
  its lines, y or Command-C copies a value, and s saves the file with the name of the
  object. The search field keeps its earlier searches on the Up key, and Esc stops a
  search that runs.
- A menu command and a shortcut for each function. Help > Keyboard Shortcuts lists them
  beside the terminal keys, with the cache location and size.
- A path bar shows how you reached the open page. Back and Forward move through it.
