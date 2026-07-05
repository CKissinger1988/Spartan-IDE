const API_BASE = 'http://localhost:8421/api';

async function fetchData(endpoint) {
  try {
    const response = await fetch(`${API_BASE}${endpoint}`);
    return await response.json();
  } catch (err) {
    console.error(`Error fetching ${endpoint}:`, err);
    return null;
  }
}

function settingsRow(primary, secondary, state = "configured") {
  return `
    <div class="settings-row">
      <div>
        <strong>${primary}</strong>
        <span>${secondary}</span>
      </div>
      <em>${state}</em>
    </div>
  `;
}

async function init() {
  const clis = await fetchData('/config/clis');
  const skillsData = await fetchData('/config/skills');
  const usageData = await fetchData('/usage');
  const status = await fetchData('/status');
  const settings = await fetchData('/config/settings');
  const neuralLink = await fetchData('/config/neural-link');

  const registryRows = document.querySelector("#registryRows");
  const cliSettingsRows = document.querySelector("#cliSettingsRows");
  const skillRows = document.querySelector("#skillRows");
  const usageRows = document.querySelector("#usageRows");
  const apiKeyInputs = document.querySelector("#apiKeyInputs");

  if (settings) {
    // General Settings
    ["AGENT_DECK_BIN", "AGENTDECK_PROFILE", "Spartan_DEFAULT_GROUP", "Spartan_DEFAULT_CLI", "Spartan_DEFAULT_MODEL", "Spartan_STYLE_SOURCE"].forEach(key => {
      const input = document.querySelector(`#setting-${key}`);
      if (input) input.value = settings[key] || "";
    });

    // API Keys
    const apiKeys = [
      "OPENAI_API_KEY",
      "ANTHROPIC_API_KEY",
      "GEMINI_API_KEY",
      "GOOGLE_API_KEY",
      "GITHUB_TOKEN",
      "PERPLEXITY_API_KEY",
      "FIRECRAWL_API_KEY",
      "EXA_API_KEY",
    ];

    apiKeyInputs.innerHTML = apiKeys.map(key => `
      <div class="settings-row input-row">
        <label>
          <span>${key}</span>
          <input type="password" class="api-key-input" data-key="${key}" value="${settings[key] || ""}" placeholder="Enter key..." />
        </label>
      </div>
    `).join("");
  }

  if (neuralLink) {
    // Neural Link Settings
    document.querySelector("#setting-HUB_ROOT").value = neuralLink.hub_root || "";
    document.querySelector("#setting-QUEUE_PATH").value = neuralLink.queue_path || "";
    document.querySelector("#setting-REPORTS_DIR").value = neuralLink.reports_dir || "";
  }

  if (clis) {
    registryRows.innerHTML = clis
      .map(([id, label, command, group, accent]) => {
        const state = "configured"; 
        return `
          <div class="registry-row ${accent}">
            <div>
              <strong>${label}</strong>
              <span>${group}</span>
            </div>
            <code>${command}</code>
            <em>${state}</em>
          </div>
        `;
      })
      .join("");

    cliSettingsRows.innerHTML = clis
      .map(([id, label, command, group]) => settingsRow(label, `${command} // ${group}`))
      .join("");
  }

  if (usageData) {
    usageRows.innerHTML = Object.entries(usageData.engines)
      .map(([id, engine]) => {
        const limit = engine.is_paid ? engine.paid_limit : engine.free_limit;
        const percent = limit ? Math.min(100, (engine.usage / limit) * 100) : 0;
        const reached = limit && engine.usage >= limit;
        return `
          <div class="usage-card ${reached ? 'limit-reached' : ''}">
            <h4>${id}</h4>
            <div class="usage-bar">
              <div class="usage-progress" style="width: ${limit ? percent : 10}%;"></div>
            </div>
            <div class="usage-info">
              <span>${engine.usage} tokens</span>
              <span>${limit || 'unlimited'}</span>
            </div>
          </div>
        `;
      })
      .join("");
  }

  if (skillsData) {
    const totalSkills = skillsData.length;
    const summaryCount = document.querySelector(".settings-summary strong");
    if (summaryCount) summaryCount.textContent = totalSkills.toLocaleString();

    skillRows.innerHTML = skillsData
      .slice(0, 50) // Show first 50
      .map(([id, name, source]) => settingsRow(name, source, "imported"))
      .join("");
  }

  // Handle terminal/status updates
  if (status && status.output) {
    const terminalBody = document.querySelector(".terminal-body");
    const statusLine = document.createElement("p");
    statusLine.className = "cyan";
    statusLine.innerHTML = status.output.replace(/\n/g, '<br>');
    terminalBody.appendChild(statusLine);
  }
}

async function saveSettings(data) {
  try {
    const response = await fetch(`${API_BASE}/config/settings`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data)
    });
    const result = await response.json();
    if (result.success) {
      alert("Settings saved successfully!");
    } else {
      alert("Error saving settings: " + result.error);
    }
  } catch (err) {
    console.error("Error saving settings:", err);
    alert("Failed to save settings.");
  }
}

async function saveNeuralLink(data) {
  try {
    const response = await fetch(`${API_BASE}/config/neural-link`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data)
    });
    const result = await response.json();
    if (result.success) {
      alert("Neural Link settings saved successfully!");
    } else {
      alert("Error saving settings: " + result.error);
    }
  } catch (err) {
    console.error("Error saving settings:", err);
    alert("Failed to save settings.");
  }
}

document.querySelector("#saveGeneralSettings").addEventListener("click", () => {
  const data = {};
  ["AGENT_DECK_BIN", "AGENTDECK_PROFILE", "Spartan_DEFAULT_GROUP", "Spartan_DEFAULT_CLI", "Spartan_DEFAULT_MODEL", "Spartan_STYLE_SOURCE"].forEach(key => {
    const input = document.querySelector(`#setting-${key}`);
    if (input) data[key] = input.value;
  });
  saveSettings(data);
});

document.querySelector("#saveApiKeys").addEventListener("click", () => {
  const data = {};
  document.querySelectorAll(".api-key-input").forEach(input => {
    data[input.dataset.key] = input.value;
  });
  saveSettings(data);
});

document.querySelector("#saveFileSettings").addEventListener("click", () => {
  const data = {
    hub_root: document.querySelector("#setting-HUB_ROOT").value,
    queue_path: document.querySelector("#setting-QUEUE_PATH").value,
    reports_dir: document.querySelector("#setting-REPORTS_DIR").value
  };
  saveNeuralLink(data);
});

// UI logic
const drawer = document.querySelector("#settingsDrawer");
const cog = document.querySelector("#settingsCog");
const closeButtons = document.querySelectorAll("[data-close-settings]");
const tabButtons = document.querySelectorAll("[data-settings-tab]");
const sections = document.querySelectorAll("[data-settings-section]");

function setDrawer(open) {
  drawer.classList.toggle("open", open);
  drawer.setAttribute("aria-hidden", open ? "false" : "true");
}

cog.addEventListener("click", () => setDrawer(true));
closeButtons.forEach((button) => button.addEventListener("click", () => setDrawer(false)));

tabButtons.forEach((button) => {
  button.addEventListener("click", () => {
    const target = button.dataset.settingsTab;
    tabButtons.forEach((item) => item.classList.toggle("active", item === button));
    sections.forEach((section) => section.classList.toggle("active", section.dataset.settingsSection === target));
  });
});

init();
