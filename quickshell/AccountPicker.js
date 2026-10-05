.pragma library

function choices(accounts) {
    const sorted = accounts.slice().sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
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
