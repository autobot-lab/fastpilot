const $ = (id) => document.getElementById(id);
chrome.storage.local.get(["port", "token"]).then(({ port = 9230, token = "" }) => { $("port").value = port; $("token").value = token; });
$("save").onclick = async () => {
  await chrome.storage.local.set({ port: Number($("port").value) || 9230, token: $("token").value });
  $("saved").textContent = "saved";
};
