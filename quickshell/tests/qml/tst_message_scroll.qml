import QtQuick
import QtTest
import "../.."

TestCase {
    name: "MessageScroll"
    when: windowShown
    width: 400
    height: 400
    MessageList {
        id: list
        width: 300
        height: 200
        spacing: 8
        delegate: Item {
            required property var modelData
            width: list.width
            height: modelData.height
        }
    }
    function rows(first, last) {
        const result = [];
        for (let i = first; i <= last; i++)
            result.push({id: {local_id: String(i)}, height: 30 + (i % 5) * 20});
        return result;
    }
    function init() {
        list.messages = [];
        wait(20);
        list.followTail = true;
    }
    function test_prepend_at_tail_keeps_visible_position() {
        list.messages = rows(100, 140);
        wait(50);
        verify(list.atYEnd);
        const old = list.itemAt(5, list.contentY + 1);
        const id = old.modelData.id.local_id;
        const offset = old.y - list.contentY;
        list.messages = rows(0, 140);
        wait(50);
        const current = list.itemAt(5, list.contentY + 1);
        compare(current.modelData.id.local_id, id);
        fuzzyCompare(current.y - list.contentY, offset, 1);
    }
    function test_new_message_follows_tail() {
        list.messages = rows(100, 140);
        wait(50);
        verify(list.atYEnd);
        list.messages = rows(100, 141);
        wait(50);
        verify(list.atYEnd);
    }
    function test_new_message_keeps_position_when_reading_older_messages() {
        list.messages = rows(100, 140);
        wait(50);
        list.followTail = false;
        list.positionViewAtIndex(15, ListView.Beginning);
        list.contentY += 12;
        const old = list.itemAt(5, list.contentY + 1);
        const id = old.modelData.id.local_id;
        const offset = old.y - list.contentY;
        list.messages = rows(100, 141);
        wait(50);
        const current = list.itemAt(5, list.contentY + 1);
        compare(current.modelData.id.local_id, id);
        fuzzyCompare(current.y - list.contentY, offset, 1);
        verify(!list.followTail);
    }
    function test_prepend_keeps_visible_message_and_offset() {
        list.messages = rows(100, 140);
        wait(50);
        list.followTail = false;
        list.positionViewAtIndex(15, ListView.Beginning);
        list.contentY += 12;
        const old = list.itemAt(5, list.contentY + 1);
        verify(old !== null);
        const id = old.modelData.id.local_id;
        const offset = old.y - list.contentY;
        list.messages = rows(0, 140);
        wait(50);
        const current = list.itemAt(5, list.contentY + 1);
        verify(current !== null);
        compare(current.modelData.id.local_id, id);
        fuzzyCompare(current.y - list.contentY, offset, 1);
    }
}
