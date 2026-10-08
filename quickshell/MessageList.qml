import QtQuick

ListView {
    id: root
    property var messages: []
    property bool followTail: true
    property bool restoringPosition: false
    property int revision: 0
    model: []

    function updateMessages() {
        const generation = ++revision;
        const previous = model || [];
        // Preserve the first visible message and its offset within the viewport.
        const item = itemAt(1, contentY + 1);
        const anchorId = item ? item.modelData.id.local_id : null;
        const offset = item ? item.y - contentY : 0;
        const previousLast = previous.length ? previous[previous.length - 1].id.local_id : null;
        const nextLast = messages.length ? messages[messages.length - 1].id.local_id : null;
        const tail = followTail && (previous.length === 0 || previousLast !== nextLast);
        const index = anchorId === null ? -1
            : messages.findIndex(message => message.id.local_id === anchorId);
        const oldY = contentY;
        restoringPosition = true;
        model = messages;
        forceLayout();
        function restore() {
            if (generation !== revision)
                return;
            if (tail) {
                positionViewAtEnd();
            } else if (index >= 0) {
                positionViewAtIndex(index, ListView.Beginning);
                forceLayout();
                const anchor = itemAtIndex(index);
                if (anchor)
                    contentY = anchor.y - offset;
            } else {
                contentY = oldY;
            }
        }
        restore();
        Qt.callLater(() => {
            restore();
            if (generation === revision)
                restoringPosition = false;
        });
    }
    onMessagesChanged: updateMessages()
    onContentHeightChanged: {
        if (followTail && !restoringPosition)
            Qt.callLater(() => {
                if (followTail && !restoringPosition)
                    positionViewAtEnd();
            });
    }
    onContentYChanged: {
        if (moving && !restoringPosition)
            followTail = atYEnd;
    }
    onMovementEnded: if (!restoringPosition) followTail = atYEnd
}
