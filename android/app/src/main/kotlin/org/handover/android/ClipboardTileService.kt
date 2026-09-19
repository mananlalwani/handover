package org.handover.android

import android.content.Intent
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

class ClipboardTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        qsTile?.apply {
            state = Tile.STATE_ACTIVE
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
