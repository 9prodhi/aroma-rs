import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
get title() { return this.record?.Name; }
}
