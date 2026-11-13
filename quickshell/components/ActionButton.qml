import QtQuick
import QtQuick.Controls

Button {
    id: control

    property bool primary: false
    property bool danger: false

    implicitHeight: 38
    implicitWidth: Math.max(76, contentItem.implicitWidth + leftPadding + rightPadding)
    leftPadding: 14
    rightPadding: 14
    topPadding: 8
    bottomPadding: 8
    hoverEnabled: true
    font.pixelSize: 13
    Accessible.name: text

    contentItem: Text {
        text: control.text
        font: control.font
        textFormat: Text.PlainText
        color: !control.enabled ? "#697586" : control.danger ? "#ffb4ab" : "#edf1f7"
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    background: Rectangle {
        radius: 8
        color: !control.enabled ? "#202733" : control.down ? "#35465c" : control.primary ? (control.hovered ? "#416d9e" : "#355c88") : control.hovered ? "#2c3747" : "#232d3b"
        border.width: 1
        border.color: control.activeFocus ? "#9bc5f4" : control.primary ? "#5d85b1" : "#354154"
    }

}
