import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
load() { return getRecord(this.recordId).then(data => { this.record = data; }); }
}
