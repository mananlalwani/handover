import QtQuick
import QtQuick.Controls

TextField {
    id: control

    implicitHeight: 38
    implicitWidth: 180
    leftPadding: 12
    rightPadding: 12
    color: enabled ? "#edf1f7" : "#697586"
    placeholderTextColor: "#8291a6"
    selectionColor: "#355c88"
    selectedTextColor: "#ffffff"
    font.pixelSize: 13
    selectByMouse: true

    background: Rectangle {
        radius: 8
        color: "#131b26"
        border.color: control.activeFocus ? "#9bc5f4" : "#354154"
    }

}
