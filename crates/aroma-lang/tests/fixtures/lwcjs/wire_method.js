import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
@wire(getRecord, { recordId: '$recordId', fields: [NAME_FIELD] }) wired({data, error}) { if (data) { this.record = data; } else { this.error = error; } }
}
