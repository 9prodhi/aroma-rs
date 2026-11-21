import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
@track reset = () => { this.rows = []; };
}
