import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
set title(value) { this._title = value.trim(); }
}
