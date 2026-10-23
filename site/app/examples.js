// First-run pages. The desktop app reads this file too (src/sheets.rs), so the
// text here is the only copy. Short, neutral, and each line has an answer.

export const PAGES = [
  {
    title: "Trip Budget",
    text: `# Trip Budget
// Four nights, three currencies.
flights = $640
hotel = 4 nights * €120 in USD
food = ¥8,000 in USD
total = sum
`,
  },
  {
    title: "Groceries",
    text: `# Groceries
// This week's shop.
milk $3.40
bread $2.80
apples $4.50
total = sum
`,
  },
  {
    title: "Monthly budget",
    text: `# Monthly budget
// Food and travel come from the other example pages.
rent = $1,450
food = @groceries.total
travel = @trip.total
spent = rent + food + travel
`,
  },
  {
    title: "Cooking",
    text: `# Cooking
// Pasta for four.
pasta = 400 g
sauce = 2 cups in ml
serves = 4
each = pasta / serves
oven = 350 fahrenheit in celsius
`,
  },
  {
    title: "Time zones",
    text: `# Time zones
// A 9:30am call on the Pacific coast.
9:30am PST in London
9:30am PST in Berlin
9:30am PST in Tokyo
`,
  },
];
