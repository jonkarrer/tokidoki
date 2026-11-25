// The browser a mission drives, driven by hand (no model): a window of the app's own goes
// to a small shop (e2e/fixtures, served by run.sh), and is read, typed into and clicked by
// the numbers each reading gives. Each step answers with the page as a mission would see it.
const invoke = window.__TAURI_INTERNALS__.invoke;
const browser = (action, more = {}) => invoke("debug_browser", { action, ...more });
const numbered = (page, name) => Number(page.split("\n").find((line) => line.includes(`"${name}"`))?.match(/^\[(\d+)\]/)?.[1]);
await t.see("form.keys");

let page = await browser("open", { url: "http://localhost:1431/shop.html" });
await t.log(page);
await t.check("the page is read: its title, address and text", page.startsWith("Page: Corner Shop\nURL: http://localhost:1431/shop.html") && page.includes("Cart: 0 items"));
await t.check("its fields and buttons are numbered", numbered(page, "Search the shop") > 0 && numbered(page, "Search") > 0);
await t.check("a password field is marked, and what it holds isn't shown", /textbox "Password" \(sensitive\)/.test(page) && !page.includes("hunter2"));

page = await browser("type", { number: numbered(page, "Search the shop"), text: "oat milk", submit: true });
await t.check("typing and Enter searches, and the results are waited for", page.includes("Add oat milk to cart"));
page = await browser("click", { number: numbered(page, "Add oat milk to cart") });
await t.check("a click is taken by the page", page.includes("Cart: 1 items"));

page = await browser("type", { number: numbered(page, "Delivery day"), text: "Friday" });
await t.check("a select takes its option by name", page.includes("Delivery on Friday"));
page = await browser("type", { number: numbered(page, "Note for the driver"), text: "Leave it by the door" });
await t.check("an editable box takes text", page.includes("Leave it by the door"));

page = await browser("click", { number: numbered(page, "Your cart, in a new window") });
await t.check("a link for a new window opens in the one page", page.startsWith("Page: Your cart"));
await t.check("the order button is there to be refused by the tool", numbered(page, "Place your order") > 0);
page = await browser("back");
await t.check("back returns to the shop", page.startsWith("Page: Corner Shop"));
const gone = await browser("click", { number: 99 }).catch((error) => String(error));
await t.check("a number that isn't on the page is refused in words", gone.includes("no element [99]"));
