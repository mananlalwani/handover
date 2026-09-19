package org.handover.android

import android.content.Intent
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

class ClipboardTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        qsTile?.apply {
            state = if (HandoverForegroundService.connectionState() == "connected") {
                Tile.STATE_ACTIVE
            } else {
                Tile.STATE_UNAVAILABLE
            }
            updateTile()
        }
    }

    override fun onClick() {
        super.onClick()
        startForegroundService(
            Intent(this, HandoverForegroundService::class.java)
                .setAction(HandoverForegroundService.ACTION_SEND_CLIPBOARD),
        )
    }
}
