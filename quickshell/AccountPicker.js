.pragma library

function choices(accounts, includeOffline) {
    const connected = accounts.filter(account => account.connected && account.authenticated);
    const visible = !includeOffline && connected.length > 0 ? connected : accounts;
    const sorted = visible.slice().sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
    return sorted.map((account, index) => {
        const number = sorted.length > 1 ? " " + (index + 1) : "";
        const state = account.connected
            ? account.authenticated ? "Connected" : "Not authenticated"
            : "Offline";
        return Object.assign({}, account, {
            displayLabel: account.label + number + " · " + state
        });
    });
}

function selectedId(accounts, current) {
    if (accounts.some(account => account.id === current))
        return current;
    const preferred = accounts.find(account => account.connected && account.authenticated)
        || accounts[0];
    return preferred ? preferred.id : "";
}
