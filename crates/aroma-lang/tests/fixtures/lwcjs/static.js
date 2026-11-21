import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
static format(value) { return String(value); }
}
